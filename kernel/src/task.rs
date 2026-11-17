//! Threads do kernel: tabela de tarefas, escalonador global e troca de contexto.
//! A política é o `Composite` de `core/sched` (FIFO e EEVDF); aqui moram o estado,
//! a pilha e a área de FPU/SIMD de cada tarefa, e o caminho da preempção (SDD 15).

use alloc::alloc::{alloc_zeroed, dealloc};
use core::alloc::Layout;
use core::cell::UnsafeCell;
use core::ptr::NonNull;
use core::slice;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rustos_hal::{Cpu, MapError, VirtAddr};
use rustos_sched::{Composite, SchedError, SchedParams, Scheduler as Policy, TaskId};

use crate::frames::Frames;
use crate::interrupts;
use crate::memory::map_stack;
use crate::platform::{self, FpuSummary, PlatformSpace};
use crate::sync::{IrqLock, locks_held};

/// Tarefas ao mesmo tempo, com a ociosa e a do boot.
pub const MAX_TASKS: usize = 16;
/// A ociosa fica fora da política: roda quando ela não devolve ninguém.
const IDLE: usize = 0;
/// A do boot vira a primeira tarefa e, no fim, o shell.
const BOOT: usize = 1;
/// Vagas da classe de tempo real na política; nenhuma tarefa do kernel a usa ainda.
pub const RT_SLOTS: usize = 4;
/// Fatia pedida pelo shell: curta, para o interativo acordar na frente de quem só calcula.
const SHELL_SLICE_NS: u64 = 200_000;
/// A tarefa do boot vira o shell: classe justa, nice 0 e a fatia curta.
pub const SHELL_PARAMS: SchedParams = SchedParams::fair_with_slice(0, SHELL_SLICE_NS);
/// Páginas da pilha de uma tarefa, fora a de guarda: 32 KiB.
const STACK_PAGES: u64 = 8;
const IDLE_STACK_PAGES: u64 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Free,
    /// Reservada por `spawn`, ainda sem contexto.
    New,
    Ready,
    Running,
    /// Em `park`, esperando um `unpark`.
    Blocked,
    Sleeping,
    Exited,
}

#[derive(Debug)]
pub enum TaskError {
    NoSlot,
    NoMemory,
    Sched(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] SchedError),
    Stack(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] MapError),
    Fpu(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] platform::PlatformError),
}

/// Como nasce uma tarefa.
#[derive(Clone, Copy, Debug)]
pub struct Spawn {
    pub name: &'static str,
    pub entry: fn(usize),
    pub arg: usize,
    /// Usa FPU/SIMD: só essas pagam o salvamento e a restauração na troca.
    pub simd: bool,
    pub params: SchedParams,
}

/// Área de estado de FPU/SIMD, alocada uma vez por tarefa que a usa.
#[derive(Debug)]
struct FpuArea {
    ptr: NonNull<u8>,
    layout: Layout,
}

// SAFETY: a área é de uma tarefa só e só a troca de contexto a lê ou escreve.
unsafe impl Send for FpuArea {}

impl FpuArea {
    fn new() -> Option<Self> {
        let layout =
            Layout::from_size_align(platform::fpu_area_bytes(), platform::FPU_AREA_ALIGN).ok()?;
        // SAFETY: o tamanho não é zero.
        let ptr = NonNull::new(unsafe { alloc_zeroed(layout) })?;
        // SAFETY: `layout.size()` bytes recém-alocados e zerados, só nossos.
        let bytes = unsafe { slice::from_raw_parts_mut(ptr.as_ptr(), layout.size()) };
        platform::fpu_init_area(bytes);
        Some(Self { ptr, layout })
    }

    /// Volta ao estado de reset, para a tarefa nova que reusa o slot.
    fn reset(&mut self) {
        // SAFETY: `layout.size()` bytes alocados em `new` e só desta área.
        let bytes = unsafe { slice::from_raw_parts_mut(self.ptr.as_ptr(), self.layout.size()) };
        platform::fpu_init_area(bytes);
    }
}

impl Drop for FpuArea {
    fn drop(&mut self) {
        // SAFETY: `ptr` veio de `alloc_zeroed` com este `layout` e não é usado depois.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

#[derive(Debug)]
struct Task {
    name: &'static str,
    state: State,
    entry: Option<fn(usize)>,
    arg: usize,
    /// A pilha mapeada fica com o slot: a tarefa nova que o reusa não mapeia outra.
    stack_top: Option<VirtAddr>,
    /// Quando acorda, se está em `Sleeping`.
    wake_at_ns: u64,
    /// `unpark` que chegou antes do `park`.
    permit: bool,
    cpu_ns: u64,
    /// Quando a tarefa assumiu a CPU, no relógio `now_ns`.
    since_ns: u64,
    switches: u64,
    /// A tarefa usa FPU/SIMD: a troca salva e restaura `fpu`.
    simd: bool,
    fpu: Option<FpuArea>,
}

impl Task {
    const FREE: Self = Self {
        name: "",
        state: State::Free,
        entry: None,
        arg: 0,
        stack_top: None,
        wake_at_ns: 0,
        permit: false,
        cpu_ns: 0,
        since_ns: 0,
        switches: 0,
        simd: false,
        fpu: None,
    };
}

/// O estado do escalonador: a política só conhece `TaskId` (o slot), e a tabela
/// guarda o resto. `next_wake_ns` é o acordar mais próximo dos que dormem.
#[derive(Debug)]
struct Scheduler {
    tasks: [Task; MAX_TASKS],
    policy: Composite<RT_SLOTS, MAX_TASKS>,
    current: usize,
    next_wake_ns: u64,
}

impl Scheduler {
    const fn new() -> Self {
        Self {
            tasks: [Task::FREE; MAX_TASKS],
            policy: Composite::new(),
            current: BOOT,
            next_wake_ns: u64::MAX,
        }
    }

    /// O próximo disparo que o timer de um tiro precisa: o fim da fatia da corrente
    /// ou o próximo acordar, o que vier antes. `None`: nada o exige.
    fn next_timer_ns(&self, now: u64) -> Option<u64> {
        let slice = self.policy.next_deadline_ns(now);
        let wake = (self.next_wake_ns != u64::MAX).then_some(self.next_wake_ns);
        match (slice, wake) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Antecipa o timer se o que a política ou os que dormem pedem vem antes do armado.
    fn rearm(&self, now: u64) {
        if let Some(at) = self.next_timer_ns(now) {
            interrupts::arm_no_later_than(at);
        }
    }

    fn id(slot: usize) -> TaskId {
        TaskId::new(u32::try_from(slot).unwrap_or(0))
    }
}

/// Um `Context` por tarefa, fora do lock: a troca os usa com o lock já solto.
struct Contexts([UnsafeCell<platform::Context>; MAX_TASKS]);

// SAFETY: uma CPU só. `schedule` lê com a IRQ mascarada; `start` escreve com ela ligada,
// mas só no slot `New` (ou o da ociosa, antes de `READY`), que nenhuma troca escolhe.
unsafe impl Sync for Contexts {}

static CONTEXTS: Contexts =
    Contexts([const { UnsafeCell::new(platform::Context::empty()) }; MAX_TASKS]);
/// Uma CPU só, e o estado de troca é global (DEBT-0036).
static SCHED: IrqLock<Scheduler> = IrqLock::new(Scheduler::new());

/// Tabelas de página e frames para mapear as pilhas das tarefas novas.
struct Space {
    tables: PlatformSpace,
    frames: Frames,
}

static SPACE: IrqLock<Option<Space>> = IrqLock::new(None);
static READY: AtomicBool = AtomicBool::new(false);
/// Quem espera a entrada do teclado ou da serial: slot + 1, ou 0.
static INPUT_WAITER: AtomicUsize = AtomicUsize::new(0);

/// Adota a execução atual como a primeira tarefa, cria a ociosa e liga FPU/SIMD.
///
/// # Safety
/// Uma vez, com as interrupções e o heap prontos; contrato de `platform::active_space`.
pub unsafe fn init(frames: Frames, direct_map_offset: u64) -> Result<FpuSummary, TaskError> {
    let summary = platform::fpu_init().map_err(TaskError::Fpu)?;
    // SAFETY: o contrato desta função.
    let tables = unsafe { platform::active_space(direct_map_offset) };
    *SPACE.lock() = Some(Space { tables, frames });
    {
        let mut sched = SCHED.lock();
        let now = now();
        let boot = &mut sched.tasks[BOOT];
        boot.name = "init";
        boot.state = State::Running;
        boot.since_ns = now;
        boot.simd = false;
        sched.current = BOOT;
        let id = Scheduler::id(BOOT);
        sched
            .policy
            .add(id, SHELL_PARAMS, now)
            .map_err(TaskError::Sched)?;
        sched.policy.pick_next(now);
    }
    start(
        IDLE,
        Spawn {
            name: "idle",
            entry: idle_main,
            arg: 0,
            simd: false,
            params: SHELL_PARAMS,
        },
        IDLE_STACK_PAGES,
    )?;
    READY.store(true, Ordering::Release);
    Ok(summary)
}

/// Cria uma tarefa pronta para rodar.
pub fn spawn(config: Spawn) -> Result<TaskId, TaskError> {
    let slot = {
        let mut sched = SCHED.lock();
        let slot = (BOOT..MAX_TASKS)
            .find(|&slot| matches!(sched.tasks[slot].state, State::Free | State::Exited))
            .ok_or(TaskError::NoSlot)?;
        sched.tasks[slot].state = State::New;
        slot
    };
    if let Err(error) = start(slot, config, STACK_PAGES) {
        SCHED.lock().tasks[slot].state = State::Free;
        return Err(error);
    }
    Ok(TaskId::new(u32::try_from(slot).unwrap_or(0)))
}

/// Dá pilha, área de FPU e contexto ao slot reservado e o põe como pronto.
fn start(slot: usize, config: Spawn, stack_pages: u64) -> Result<(), TaskError> {
    let (known_stack, has_fpu) = {
        let sched = SCHED.lock();
        (sched.tasks[slot].stack_top, sched.tasks[slot].fpu.is_some())
    };
    let top = if let Some(top) = known_stack {
        top
    } else {
        let mut space = SPACE.lock();
        let space = space.as_mut().ok_or(TaskError::NoMemory)?;
        map_stack(&mut space.frames, &mut space.tables, stack_pages).map_err(TaskError::Stack)?
    };
    let area = if config.simd && !has_fpu {
        Some(FpuArea::new().ok_or(TaskError::NoMemory)?)
    } else {
        None
    };
    // SAFETY: `top` é o fim de uma pilha mapeada, alinhada a 16 e só desta tarefa.
    let context = unsafe { platform::Context::new(top, task_entry, slot, task_exit) };
    // SAFETY: o slot está reservado e nunca rodou, então ninguém lê o contexto.
    unsafe { *CONTEXTS.0[slot].get() = context };
    let mut sched = SCHED.lock();
    let task = &mut sched.tasks[slot];
    task.name = config.name;
    task.entry = Some(config.entry);
    task.arg = config.arg;
    task.stack_top = Some(top);
    task.permit = false;
    task.cpu_ns = 0;
    task.switches = 0;
    task.simd = config.simd;
    if area.is_some() {
        task.fpu = area;
    } else if let Some(existing) = task.fpu.as_mut() {
        existing.reset();
    }
    if slot != IDLE {
        let now = now();
        sched
            .policy
            .add(Scheduler::id(slot), config.params, now)
            .map_err(TaskError::Sched)?;
        sched.rearm(now);
    }
    sched.tasks[slot].state = State::Ready;
    Ok(())
}

/// A primeira instrução de toda tarefa nova, depois do trampolim de `arch/`.
extern "C" fn task_entry(slot: usize) {
    platform::cpu().restore_interrupts(true);
    let (entry, arg) = {
        let sched = SCHED.lock();
        (sched.tasks[slot].entry, sched.tasks[slot].arg)
    };
    if let Some(entry) = entry {
        entry(arg);
    }
}

/// Para onde o trampolim vai quando a função da tarefa retorna.
extern "C" fn task_exit() -> ! {
    exit()
}

/// A tarefa ociosa: dorme até a próxima interrupção e cede a CPU a quem estiver pronto.
fn idle_main(_: usize) {
    let cpu = platform::cpu();
    loop {
        cpu.save_and_disable_interrupts();
        if has_ready() {
            cpu.restore_interrupts(true);
        } else {
            cpu.enable_interrupts_and_wait();
        }
        yield_now();
    }
}

/// A ociosa roda: a política pede troca se alguém ficou pronto.
fn has_ready() -> bool {
    SCHED.lock().policy.needs_resched()
}

fn now() -> u64 {
    platform::now_ns().unwrap_or(0)
}

/// Escolhe a próxima tarefa e troca para ela. Chamar com a interrupção mascarada
/// e sem `IrqLock` preso: o lock do escalonador é solto antes da troca.
fn schedule() {
    schedule_at(now());
}

/// `schedule` com o relógio que o chamador já leu: a leitura custa de 15 a 25 ns no HVF.
fn schedule_at(now_ns: u64) {
    debug_assert_eq!(locks_held(), 0, "troca de contexto com IrqLock preso");
    let (from, to, save, load) = {
        let mut sched = SCHED.lock();
        let from = sched.current;
        let outgoing = &mut sched.tasks[from];
        outgoing.cpu_ns += now_ns.saturating_sub(outgoing.since_ns);
        if outgoing.state == State::Running {
            outgoing.state = State::Ready;
        }
        let to = sched
            .policy
            .pick_next(now_ns)
            .map_or(IDLE, |id| id.as_u32() as usize);
        let incoming = &mut sched.tasks[to];
        incoming.state = State::Running;
        incoming.since_ns = now_ns;
        if to != from {
            incoming.switches += 1;
        }
        sched.rearm(now_ns);
        if to == from {
            return;
        }
        sched.current = to;
        let save = fpu_pointer(&sched.tasks[from]);
        let load = fpu_pointer(&sched.tasks[to]);
        (from, to, save, load)
    };
    if let Some(area) = save {
        // SAFETY: a área é da tarefa que sai, viva, e tem o tamanho que `fpu_init` mediu.
        unsafe { platform::fpu_save(area.as_ptr()) };
    }
    if let Some(area) = load {
        // SAFETY: a área é da tarefa que entra, viva, e foi preparada ou salva antes.
        unsafe { platform::fpu_restore(area.as_ptr()) };
    }
    // SAFETY: `to` foi preparada por `start` ou guardada por uma troca anterior;
    // a interrupção está mascarada e nenhum lock está preso.
    unsafe { platform::switch(CONTEXTS.0[from].get(), CONTEXTS.0[to].get()) };
}

/// A área de FPU de uma tarefa que a usa e que ainda tem de ser salva.
fn fpu_pointer(task: &Task) -> Option<NonNull<u8>> {
    if task.simd && task.state != State::Exited {
        task.fpu.as_ref().map(|area| area.ptr)
    } else {
        None
    }
}

/// Cede a CPU a outra tarefa pronta, se houver.
pub fn yield_now() {
    assert_task_context();
    let cpu = platform::cpu();
    let were_enabled = cpu.save_and_disable_interrupts();
    let now = now();
    {
        let mut sched = SCHED.lock();
        if sched.current != IDLE {
            sched.policy.yield_current(now);
        }
    }
    schedule_at(now);
    cpu.restore_interrupts(were_enabled);
}

/// Dorme até o relógio (`now_ns`) chegar a `deadline_ns`.
pub fn sleep_until(deadline_ns: u64) {
    assert_task_context();
    let cpu = platform::cpu();
    let were_enabled = cpu.save_and_disable_interrupts();
    let now = now();
    if now < deadline_ns {
        {
            let mut sched = SCHED.lock();
            let current = sched.current;
            let task = &mut sched.tasks[current];
            task.wake_at_ns = deadline_ns;
            task.state = State::Sleeping;
            sched.next_wake_ns = sched.next_wake_ns.min(deadline_ns);
            sched.policy.sleep(Scheduler::id(current), now);
        }
        schedule_at(now);
    }
    cpu.restore_interrupts(were_enabled);
}

/// Espera um `unpark`; volta já se ele chegou antes.
pub fn park() {
    assert_task_context();
    let cpu = platform::cpu();
    let were_enabled = cpu.save_and_disable_interrupts();
    park_masked();
    cpu.restore_interrupts(were_enabled);
}

/// `park` para quem já decidiu com a interrupção mascarada, sem janela para perder o acordar.
pub fn park_masked() {
    let now = now();
    {
        let mut sched = SCHED.lock();
        let current = sched.current;
        let task = &mut sched.tasks[current];
        if core::mem::take(&mut task.permit) {
            return;
        }
        task.state = State::Blocked;
        sched.policy.sleep(Scheduler::id(current), now);
    }
    schedule_at(now);
}

/// Acorda a tarefa de `slot`, ou deixa o aviso para o próximo `park` dela.
/// Slot que não está vivo (`Free`, `New`, `Exited`) ignora o aviso.
pub fn unpark(id: TaskId) {
    let cpu = platform::cpu();
    let were_enabled = cpu.save_and_disable_interrupts();
    {
        let mut sched = SCHED.lock();
        let task = &mut sched.tasks[id.as_u32() as usize];
        match task.state {
            State::Blocked => {
                task.state = State::Ready;
                let now = now();
                sched.policy.wake(id, now);
                sched.rearm(now);
            }
            State::Free | State::New | State::Exited => {}
            _ => task.permit = true,
        }
    }
    cpu.restore_interrupts(were_enabled);
}

/// Encerra a tarefa atual; o slot e a pilha ficam para a próxima.
pub fn exit() -> ! {
    assert_task_context();
    platform::cpu().save_and_disable_interrupts();
    {
        let mut sched = SCHED.lock();
        let current = sched.current;
        sched.tasks[current].state = State::Exited;
        sched.policy.remove(Scheduler::id(current));
    }
    schedule();
    unreachable!("tarefa encerrada voltou a rodar");
}

fn assert_task_context() {
    debug_assert!(
        !interrupts::in_irq(),
        "yield, sleep ou park dentro de uma interrupção"
    );
    debug_assert!(READY.load(Ordering::Relaxed), "escalonador sem `init`");
}

/// A tarefa que roda agora.
pub fn current() -> TaskId {
    TaskId::new(u32::try_from(SCHED.lock().current).unwrap_or(0))
}

pub fn set_current_name(name: &'static str) {
    let mut sched = SCHED.lock();
    let current = sched.current;
    sched.tasks[current].name = name;
}

/// A tarefa terminou e o slot está livre para a próxima.
pub fn has_exited(id: TaskId) -> bool {
    SCHED.lock().tasks[id.as_u32() as usize].state == State::Exited
}

/// Quantas vezes a tarefa foi escolhida para rodar.
pub fn switches(id: TaskId) -> u64 {
    SCHED.lock().tasks[id.as_u32() as usize].switches
}

/// Chamado da IRQ do timer: acorda quem dorme com prazo vencido, cobra a corrente
/// e devolve o próximo disparo que o escalonador pede (`None`: nenhum, sem tique).
pub fn timer_irq(now_ns: u64) -> Option<u64> {
    let mut sched = SCHED.lock();
    let mut next = u64::MAX;
    for slot in 0..MAX_TASKS {
        let task = &mut sched.tasks[slot];
        if task.state != State::Sleeping {
            continue;
        }
        if task.wake_at_ns <= now_ns {
            task.state = State::Ready;
            sched.policy.wake(Scheduler::id(slot), now_ns);
        } else {
            next = next.min(task.wake_at_ns);
        }
    }
    sched.next_wake_ns = next;
    sched.policy.tick(now_ns);
    sched.next_timer_ns(now_ns)
}

/// O escalonador já tem a política ligada; antes disso o tique periódico manda.
pub fn is_ready() -> bool {
    READY.load(Ordering::Acquire)
}

/// Chamado pelo `arch/` no fim de toda IRQ, depois do EOI, com a interrupção
/// mascarada e na pilha da tarefa interrompida: o quadro dela sobrevive à troca.
pub fn preempt() {
    if READY.load(Ordering::Acquire) && SCHED.lock().policy.needs_resched() {
        schedule();
    }
}

/// Registra a tarefa atual como a que espera entrada, para a IRQ acordá-la.
pub fn wait_for_input() {
    let current = SCHED.lock().current;
    INPUT_WAITER.store(current + 1, Ordering::Relaxed);
    park_masked();
}

/// Chamado da IRQ quando entrou byte na fila de entrada.
pub fn notify_input() {
    let waiter = INPUT_WAITER.swap(0, Ordering::Relaxed);
    if waiter != 0 {
        let id = TaskId::new(u32::try_from(waiter - 1).unwrap_or(0));
        unpark(id);
    }
}

/// O que `ps` mostra de uma tarefa.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub id: u32,
    pub name: &'static str,
    pub state: State,
    pub cpu_ns: u64,
    pub switches: u64,
    pub simd: bool,
}

/// Retrato das tarefas vivas, tirado sob o lock e entregue fora dele.
pub fn snapshot() -> [Option<Snapshot>; MAX_TASKS] {
    let mut out = [None; MAX_TASKS];
    let sched = SCHED.lock();
    let now = now();
    for (slot, task) in sched.tasks.iter().enumerate() {
        if matches!(task.state, State::Free | State::Exited | State::New) {
            continue;
        }
        let running = if slot == sched.current {
            now.saturating_sub(task.since_ns)
        } else {
            0
        };
        out[slot] = Some(Snapshot {
            id: u32::try_from(slot).unwrap_or(0),
            name: task.name,
            state: task.state,
            cpu_ns: task.cpu_ns + running,
            switches: task.switches,
            simd: task.simd,
        });
    }
    out
}
