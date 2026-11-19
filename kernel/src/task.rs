//! Threads do kernel: tabela de tarefas, uma fila de execução por CPU e a troca de contexto.
//! A política de cada fila é o `Composite` de `core/sched`; aqui moram o estado, a pilha e a
//! área de FPU/SIMD de cada tarefa, a migração entre CPUs e a preempção (SDD 15).

use alloc::alloc::{alloc_zeroed, dealloc};
use alloc::boxed::Box;
use core::alloc::Layout;
use core::cell::UnsafeCell;
use core::ptr::NonNull;
use core::slice;
use core::sync::atomic::{
    AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering, fence,
};

use rustos_hal::{Cpu, Ipi, IpiKind, MAX_CPUS, MapError};
use rustos_sched::{Composite, SchedError, SchedParams, Scheduler, TaskId};

#[cfg(feature = "selftest")]
use crate::bench;
use crate::frames::Frames;
use crate::interrupts;
use crate::memory::map_stack_painted;
use crate::percpu::{self, NO_TASK, PerCpu};
use crate::platform::{self, FpuSummary, PlatformSpace};
use crate::stack::Stack;
use crate::sync::{IrqGuard, IrqLock, bump, locks_held};

/// Os slots `0..MAX_CPUS` são as ociosas, uma por CPU; os 24 seguintes, das tarefas.
pub const MAX_TASKS: usize = MAX_CPUS + 24;
const FIRST_TASK: usize = MAX_CPUS;
/// Vagas da classe de tempo real na política; nenhuma tarefa do kernel a usa ainda.
pub const RT_SLOTS: usize = 4;
type Policy = Composite<RT_SLOTS, MAX_TASKS>;
/// Fatia pedida pelo shell: curta, para o interativo acordar na frente de quem só calcula.
const SHELL_SLICE_NS: u64 = 200_000;
/// A tarefa do boot vira o shell: classe justa, nice 0 e a fatia curta.
pub const SHELL_PARAMS: SchedParams = SchedParams::fair_with_slice(0, SHELL_SLICE_NS);
/// Páginas da pilha de toda thread do kernel, fora a do boot e a de guarda: 16 KiB (SDD 15).
pub const STACK_PAGES: u64 = 4;
/// Intervalo mínimo entre dois pulls periódicos de uma CPU.
const BALANCE_NS: u64 = 2_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
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

impl State {
    const fn from_u8(raw: u8) -> Self {
        match raw {
            1 => Self::New,
            2 => Self::Ready,
            3 => Self::Running,
            4 => Self::Blocked,
            5 => Self::Sleeping,
            6 => Self::Exited,
            _ => Self::Free,
        }
    }
}

#[derive(Debug)]
pub enum TaskError {
    NoSlot,
    NoMemory,
    /// A máscara de afinidade não tem nenhuma CPU ativa.
    NoCpu,
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
    /// CPUs em que ela pode rodar, um bit por CPU; zero é qualquer uma.
    pub affinity: u32,
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

/// O que só muda no `spawn`: ninguém o lê enquanto o slot é `New`.
#[derive(Debug)]
struct Meta {
    name: &'static str,
    entry: Option<fn(usize)>,
    arg: usize,
    /// A pilha mapeada fica com o slot: a tarefa nova que o reusa não mapeia outra.
    stack: Option<Stack>,
    fpu: Option<FpuArea>,
}

/// Uma tarefa. `state`, `permit` e a presença na política só mudam sob o lock da fila da CPU
/// em `cpu`; `on_cpu` sobe sob esse lock e desce na CPU que a deixou, depois da troca.
#[derive(Debug)]
struct Slot {
    state: AtomicU8,
    /// A fila dona da tarefa, mesmo dormindo: só muda sob o lock dela ou de quem a destacou.
    cpu: AtomicU32,
    /// O contexto ainda é usado por uma CPU (rodando, ou saindo da troca): nenhuma outra
    /// CPU a escolhe, migra nem reusa o slot enquanto isto vale.
    on_cpu: AtomicBool,
    /// `unpark` que chegou antes do `park`.
    permit: AtomicBool,
    affinity: AtomicU32,
    /// Quando acorda, se está em `Sleeping`.
    wake_at_ns: AtomicU64,
    cpu_ns: AtomicU64,
    /// Quando a tarefa assumiu a CPU, no relógio do escalonador (`now`).
    since_ns: AtomicU64,
    switches: AtomicU64,
    /// Endereço da área de FPU/SIMD; zero se a tarefa não usa SIMD.
    fpu: AtomicUsize,
    /// A raiz de página do processo de usuário que a tarefa roda; zero na tarefa só de kernel.
    user_root: AtomicU64,
    /// O topo da pilha do kernel da tarefa de usuário (a pilha de entrada de syscall e exceção).
    user_top: AtomicU64,
    /// A thread de usuário da tarefa (`Arc::into_raw` de um `ThreadObj`), ou zero.
    process: AtomicUsize,
    /// A página da última falta que só repetiu (`Retry`), mais um; zero se a última não repetiu.
    retry_page: AtomicU64,
    /// O endereço da máscara de CPUs do espaço que a tarefa roda (`UserSpace::active_addr`), ou zero.
    user_active: AtomicUsize,
    meta: IrqLock<Meta>,
}

impl Slot {
    const fn new() -> Self {
        Self {
            state: AtomicU8::new(State::Free as u8),
            cpu: AtomicU32::new(0),
            on_cpu: AtomicBool::new(false),
            permit: AtomicBool::new(false),
            affinity: AtomicU32::new(0),
            wake_at_ns: AtomicU64::new(u64::MAX),
            cpu_ns: AtomicU64::new(0),
            since_ns: AtomicU64::new(0),
            switches: AtomicU64::new(0),
            fpu: AtomicUsize::new(0),
            user_root: AtomicU64::new(0),
            user_top: AtomicU64::new(0),
            process: AtomicUsize::new(0),
            retry_page: AtomicU64::new(0),
            user_active: AtomicUsize::new(0),
            meta: IrqLock::new(Meta {
                name: "",
                entry: None,
                arg: 0,
                stack: None,
                fpu: None,
            }),
        }
    }

    fn state(&self) -> State {
        State::from_u8(self.state.load(Ordering::Acquire))
    }

    fn set_state(&self, state: State) {
        self.state.store(state as u8, Ordering::Release);
    }

    fn may_run_on(&self, cpu: usize) -> bool {
        let mask = self.affinity.load(Ordering::Relaxed);
        mask == 0 || mask >> cpu & 1 != 0
    }
}

/// A máscara de espera de um objeto tem um bit por slot de tarefa.
const _: () = assert!(MAX_TASKS <= 64);

static TASKS: [Slot; MAX_TASKS] = [const { Slot::new() }; MAX_TASKS];

fn slot(id: usize) -> &'static Slot {
    &TASKS[id]
}

fn id_of(slot: usize) -> TaskId {
    TaskId::new(u32::try_from(slot).unwrap_or(0))
}

fn slot_u32(slot: usize) -> u32 {
    u32::try_from(slot).unwrap_or(0)
}

/// A fila de uma CPU: a política, a ociosa dela e o acordar mais próximo de quem dorme nela.
#[derive(Debug)]
pub struct Queue {
    policy: Option<Box<Policy>>,
    idle: usize,
    next_wake_ns: u64,
}

impl Queue {
    pub const fn new() -> Self {
        Self {
            policy: None,
            idle: 0,
            next_wake_ns: u64::MAX,
        }
    }

    fn policy(&mut self) -> &mut Policy {
        self.policy.as_deref_mut().expect("fila sem política")
    }

    /// Alguém pronto espera a CPU, ou a corrente já devia trocar.
    fn wants_resched(&self) -> bool {
        self.policy.as_deref().is_some_and(Scheduler::needs_resched)
    }

    /// O próximo disparo que o timer de um tiro precisa: o fim da fatia da corrente ou o
    /// próximo acordar, o que vier antes. `None`: nada o exige.
    fn next_timer_ns(&self, now: u64) -> Option<u64> {
        let slice = self
            .policy
            .as_deref()
            .and_then(|policy| policy.next_deadline_ns(now));
        let wake = (self.next_wake_ns != u64::MAX).then_some(self.next_wake_ns);
        match (slice, wake) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Antecipa o timer desta CPU se o que a política ou os que dormem pedem vem antes do armado.
    fn rearm(&self, now: u64) {
        if let Some(at) = self.next_timer_ns(now) {
            interrupts::arm_no_later_than(at);
        }
    }
}

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}

/// Um `Context` por tarefa, fora do lock: a troca os usa com o lock já solto.
struct Contexts([UnsafeCell<platform::Context>; MAX_TASKS]);

// SAFETY: `schedule` lê com a IRQ mascarada só os de quem sai e de quem entra, que nenhuma outra
// CPU usa (`on_cpu`); `start` escreve só no slot `New`, que nenhuma fila contém.
unsafe impl Sync for Contexts {}

static CONTEXTS: Contexts =
    Contexts([const { UnsafeCell::new(platform::Context::empty()) }; MAX_TASKS]);

/// Tabelas de página e frames para mapear as pilhas das tarefas novas.
struct Space {
    tables: PlatformSpace,
    frames: Frames,
}

static SPACE: IrqLock<Option<Space>> = IrqLock::new(None);
/// CPUs cuja fila existe: só elas recebem tarefa.
static ACTIVE: AtomicU32 = AtomicU32::new(0);
/// Quem espera a entrada do teclado ou da serial: slot + 1, ou 0.
static INPUT_WAITER: AtomicUsize = AtomicUsize::new(0);
/// O maior pico das tarefas cujo slot já foi reusado: o pico some ao repintar a pilha.
static TASK_PEAK: AtomicUsize = AtomicUsize::new(0);
/// A pilha do boot, que a tarefa `init` adota.
static BOOT_STACK: IrqLock<Option<Stack>> = IrqLock::new(None);

/// Guarda a pilha do boot para `init` adotá-la como a do shell.
pub fn adopt_boot_stack(stack: Stack) {
    *BOOT_STACK.lock() = Some(stack);
}

/// Adota a execução atual como a primeira tarefa da CPU de boot, cria a ociosa dela e liga
/// FPU/SIMD.
/// # Safety
/// Uma vez, com as interrupções e o heap prontos; contrato de `platform::active_space`.
pub unsafe fn init(frames: Frames, direct_map_offset: u64) -> Result<FpuSummary, TaskError> {
    let summary = platform::fpu_init().map_err(TaskError::Fpu)?;
    // SAFETY: o contrato desta função.
    let tables = unsafe { platform::active_space(direct_map_offset) };
    let mut user_frames = frames;
    platform::user_init(&mut user_frames).map_err(TaskError::Stack)?;
    *SPACE.lock() = Some(Space { tables, frames });
    let cpu = percpu::current();
    let policy = Box::new(Policy::new());
    let boot = FIRST_TASK;
    {
        let mut queue = cpu.queue.lock();
        queue.policy = Some(policy);
        queue.idle = 0;
        let now = now();
        let task = slot(boot);
        {
            let mut meta = task.meta.lock();
            meta.name = "init";
            meta.stack = *BOOT_STACK.lock();
        }
        task.cpu.store(0, Ordering::Release);
        task.affinity.store(0, Ordering::Relaxed);
        task.set_state(State::Running);
        task.on_cpu.store(true, Ordering::Release);
        task.since_ns.store(now, Ordering::Relaxed);
        cpu.current.store(slot_u32(boot), Ordering::Relaxed);
        let policy = queue.policy();
        policy
            .add(id_of(boot), SHELL_PARAMS, now)
            .map_err(TaskError::Sched)?;
        policy.pick_next(now);
        publish(cpu, &queue);
    }
    start(
        0,
        Spawn {
            name: "idle",
            entry: idle_main,
            arg: 0,
            simd: false,
            params: SHELL_PARAMS,
            affinity: 1,
        },
    )?;
    ACTIVE.fetch_or(1, Ordering::Release);
    Ok(summary)
}

/// Entra no escalonador numa CPU secundária: a execução atual vira a ociosa dela e não volta.
pub fn run_secondary(stack: Stack) -> ! {
    platform::user_cpu_init();
    let cpu = percpu::current();
    let index = cpu.index();
    let policy = Box::new(Policy::new());
    {
        let mut queue = cpu.queue.lock();
        queue.policy = Some(policy);
        queue.idle = index;
        let idle = slot(index);
        {
            let mut meta = idle.meta.lock();
            meta.name = "idle";
            meta.stack = Some(stack);
        }
        idle.cpu.store(slot_u32(index), Ordering::Release);
        idle.affinity.store(1 << index, Ordering::Relaxed);
        idle.set_state(State::Running);
        idle.on_cpu.store(true, Ordering::Release);
        idle.since_ns.store(now(), Ordering::Relaxed);
        cpu.current.store(slot_u32(index), Ordering::Relaxed);
    }
    ACTIVE.fetch_or(1 << index, Ordering::Release);
    idle_loop()
}

/// Publica a carga da fila para as outras CPUs; chamar com o lock da fila preso.
fn publish(cpu: &PerCpu, queue: &Queue) {
    let load = queue.policy.as_deref().map_or(0, Policy::runnable);
    cpu.load
        .store(u32::try_from(load).unwrap_or(u32::MAX), Ordering::Relaxed);
}

/// Cria uma tarefa pronta para rodar, na CPU menos carregada que a afinidade permite.
/// Se a nova deve rodar antes da corrente, a troca vem no fim da chamada (`preempt_now`).
pub fn spawn(config: Spawn) -> Result<TaskId, TaskError> {
    let id = claim_slot().ok_or(TaskError::NoSlot)?;
    match start(id, config) {
        Ok(preempts) => {
            if preempts {
                let cpu = platform::cpu();
                let were_enabled = cpu.save_and_disable_interrupts();
                preempt_now(were_enabled, now());
                cpu.restore_interrupts(were_enabled);
            }
            Ok(id_of(id))
        }
        Err(error) => {
            slot(id).set_state(State::Free);
            Err(error)
        }
    }
}

/// Reserva um slot livre. O de uma tarefa que saiu só vale depois que a CPU dela terminou
/// a troca (`on_cpu` baixo): até lá a pilha e o contexto ainda estão em uso.
fn claim_slot() -> Option<usize> {
    for id in FIRST_TASK..MAX_TASKS {
        let candidate = slot(id);
        let seen = candidate.state.load(Ordering::Acquire);
        let free = seen == State::Free as u8
            || seen == State::Exited as u8 && !candidate.on_cpu.load(Ordering::Acquire);
        if free
            && candidate
                .state
                .compare_exchange(seen, State::New as u8, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
        {
            return Some(id);
        }
    }
    None
}

/// Dá pilha, área de FPU e contexto ao slot reservado e o põe numa fila, pronto.
fn start(id: usize, config: Spawn) -> Result<bool, TaskError> {
    let target = slot(id);
    let (known_stack, has_fpu) = {
        let meta = target.meta.lock();
        (meta.stack, meta.fpu.is_some())
    };
    let stack = if let Some(stack) = known_stack {
        // SAFETY: o slot é `New` com `on_cpu` baixo: nenhuma CPU roda nesta pilha.
        let used = unsafe { stack.repaint() };
        if id >= FIRST_TASK {
            TASK_PEAK.fetch_max(used, Ordering::Relaxed);
        }
        stack
    } else {
        let mut space = SPACE.lock();
        let space = space.as_mut().ok_or(TaskError::NoMemory)?;
        map_stack_painted(&mut space.frames, &mut space.tables, STACK_PAGES)
            .map_err(TaskError::Stack)?
    };
    let top = stack.top();
    let area = if config.simd && !has_fpu {
        Some(FpuArea::new().ok_or(TaskError::NoMemory)?)
    } else {
        None
    };
    // SAFETY: `top` é o fim de uma pilha mapeada, alinhada a 16 e só desta tarefa.
    let context = unsafe { platform::Context::new(top, task_entry, id, task_exit) };
    // SAFETY: o slot está reservado (`New`) e com `on_cpu` baixo, então ninguém lê o contexto.
    unsafe { *CONTEXTS.0[id].get() = context };
    {
        let mut meta = target.meta.lock();
        meta.name = config.name;
        meta.entry = Some(config.entry);
        meta.arg = config.arg;
        meta.stack = Some(stack);
        if area.is_some() {
            meta.fpu = area;
        } else if let Some(existing) = meta.fpu.as_mut() {
            existing.reset();
        }
        let area = if config.simd {
            meta.fpu.as_ref().map_or(0, |area| area.ptr.as_ptr().addr())
        } else {
            0
        };
        target.fpu.store(area, Ordering::Relaxed);
    }
    target.permit.store(false, Ordering::Relaxed);
    target.user_root.store(0, Ordering::Relaxed);
    target.user_top.store(0, Ordering::Relaxed);
    target.process.store(0, Ordering::Relaxed);
    target.retry_page.store(0, Ordering::Relaxed);
    target.user_active.store(0, Ordering::Relaxed);
    target.cpu_ns.store(0, Ordering::Relaxed);
    target.switches.store(0, Ordering::Relaxed);
    target.wake_at_ns.store(u64::MAX, Ordering::Relaxed);
    target.affinity.store(config.affinity, Ordering::Relaxed);
    if id < FIRST_TASK {
        target.cpu.store(slot_u32(id), Ordering::Release);
        target.set_state(State::Ready);
        return Ok(false);
    }
    let cpu_index = pick_cpu(config.affinity, None).ok_or(TaskError::NoCpu)?;
    target.cpu.store(slot_u32(cpu_index), Ordering::Release);
    let cpu = percpu::get(cpu_index);
    let now = now();
    let mut queue = cpu.queue.lock();
    queue
        .policy()
        .add(id_of(id), config.params, now)
        .map_err(TaskError::Sched)?;
    target.set_state(State::Ready);
    publish(cpu, &queue);
    Ok(notify(cpu, &queue, now))
}

/// A CPU menos carregada entre as ativas que a máscara permite; empata a favor de `prefer`.
/// Lê a carga sem lock: é uma estimativa, e o pull corrige o que ela errar.
fn pick_cpu(affinity: u32, prefer: Option<usize>) -> Option<usize> {
    let mask = if affinity == 0 { u32::MAX } else { affinity };
    let allowed = ACTIVE.load(Ordering::Acquire) & mask;
    let mut best: Option<(u32, usize)> = None;
    for index in 0..MAX_CPUS {
        if allowed >> index & 1 == 0 {
            continue;
        }
        let load = percpu::get(index).load.load(Ordering::Relaxed);
        let better = match best {
            None => true,
            Some((least, _)) => load < least || load == least && prefer == Some(index),
        };
        if better {
            best = Some((load, index));
        }
    }
    best.map(|(_, index)| index)
}

/// Depois de pôr uma tarefa na fila de `target`: se a CPU é outra e precisa refazer a decisão
/// (a corrente devia trocar, ou há prazo mais cedo que o armado), manda o IPI. Lock da fila preso.
/// `true`: a fila é desta CPU e a corrente devia ceder a vez já (o chamador vê `preempt_now`).
fn notify(target: &PerCpu, queue: &Queue, now: u64) -> bool {
    if target.index() == percpu::current().index() {
        queue.rearm(now);
        return queue.wants_resched();
    }
    let sooner = queue
        .next_timer_ns(now)
        .is_some_and(|at| at < target.armed_ns());
    if queue.wants_resched() || sooner {
        kick(target);
    }
    false
}

/// Manda o IPI de reescalonar, se não há um a caminho.
fn kick(target: &PerCpu) {
    if !target.kick_pending.swap(true, Ordering::AcqRel) {
        bump(&percpu::current().stats.ipis_sent, 1);
        // SAFETY: há outra CPU ativa, então a interface de interrupção está ligada.
        unsafe { platform::ipi() }.send(target.hw_id(), IpiKind::Reschedule);
    }
}

/// O IPI chegou a esta CPU: libera o próximo e refaz o prazo do timer. A troca, se for o caso,
/// vem em `preempt`, no fim da IRQ.
pub fn on_reschedule_ipi() {
    let cpu = percpu::current();
    cpu.kick_pending.store(false, Ordering::Release);
    bump(&cpu.stats.ipis_received, 1);
    #[cfg(feature = "selftest")]
    bench::on_ipi();
    if is_ready() {
        cpu.queue.lock().rearm(now());
    }
}

/// A primeira instrução de toda tarefa nova, depois do trampolim de `arch/`.
extern "C" fn task_entry(id: usize) {
    finish_switch();
    platform::cpu().restore_interrupts(true);
    let (entry, arg) = {
        let meta = slot(id).meta.lock();
        (meta.entry, meta.arg)
    };
    if let Some(entry) = entry {
        entry(arg);
    }
}

/// Para onde o trampolim vai quando a função da tarefa retorna.
extern "C" fn task_exit() -> ! {
    exit()
}

/// A tarefa ociosa da CPU de boot.
fn idle_main(_: usize) {
    idle_loop()
}

/// A ociosa de uma CPU. Antes de dormir tenta puxar trabalho de outra fila; depois de cada
/// interrupção (o IPI de reescalonar também) confere de novo.
fn idle_loop() -> ! {
    let cpu = platform::cpu();
    loop {
        cpu.save_and_disable_interrupts();
        let here = percpu::current();
        if here.queue.lock().wants_resched() || pull(here.index(), 2) {
            cpu.restore_interrupts(true);
        } else {
            here.idle.store(true, Ordering::Relaxed);
            cpu.enable_interrupts_and_wait();
            here.idle.store(false, Ordering::Relaxed);
        }
        yield_now();
    }
}

fn now() -> u64 {
    platform::sched_clock_ns().unwrap_or(0)
}

/// A CPU `thief` puxa uma tarefa pronta da CPU mais carregada, se ela tem pelo menos `gap`
/// a mais. Nunca segura duas filas: destaca sob a da vítima, solta, e só então anexa sob a
/// sua. A tarefa em trânsito é de quem a destacou (`cpu` já aponta para cá, estado `Ready`).
fn pull(thief: usize, gap: u32) -> bool {
    let me = percpu::get(thief);
    let mine = me.load.load(Ordering::Relaxed);
    let active = ACTIVE.load(Ordering::Acquire);
    let mut victim: Option<(u32, usize)> = None;
    for index in 0..MAX_CPUS {
        if index == thief || active >> index & 1 == 0 {
            continue;
        }
        let load = percpu::get(index).load.load(Ordering::Relaxed);
        if load >= mine + gap && victim.is_none_or(|(most, _)| load > most) {
            victim = Some((load, index));
        }
    }
    let Some((_, victim)) = victim else {
        return false;
    };
    let now = now();
    let source = percpu::get(victim);
    let (task, detached) = {
        let mut queue = source.queue.lock();
        let candidate = queue.policy().pull_candidate(|task| {
            let candidate = slot(task.as_u32() as usize);
            !candidate.on_cpu.load(Ordering::Acquire) && candidate.may_run_on(thief)
        });
        let Some(task) = candidate else {
            return false;
        };
        let Some(detached) = queue.policy().detach(task) else {
            return false;
        };
        slot(task.as_u32() as usize)
            .cpu
            .store(slot_u32(thief), Ordering::Release);
        publish(source, &queue);
        (task, detached)
    };
    let mut queue = me.queue.lock();
    if let Err(error) = queue.policy().add_with_lag(task, detached, now) {
        panic!("migração perdeu a tarefa {}: {error:?}", task.as_u32());
    }
    publish(me, &queue);
    bump(&me.stats.pulled, 1);
    true
}

/// Lock preso na troca é deadlock mudo: a conferência vale também em `release` (SDD 15).
#[inline]
fn check_locks_held(expected: u32) {
    let held = locks_held();
    if held != expected {
        locks_held_on_switch(held, expected);
    }
}

#[cold]
#[inline(never)]
fn locks_held_on_switch(held: u32, expected: u32) -> ! {
    panic!("context switch with {held} IrqLock held, expected {expected}");
}

/// Escolhe a próxima tarefa e troca para ela. Chamar com a interrupção mascarada
/// e sem `IrqLock` preso: o lock da fila é solto antes da troca.
fn schedule() {
    schedule_at(now());
}

/// `schedule` com o relógio que o chamador já leu.
fn schedule_at(now_ns: u64) {
    let cpu = percpu::current();
    check_locks_held(0);
    switch_from(cpu, cpu.queue.lock(), now_ns);
}

/// `schedule_at` para quem já tem o lock da fila preso e mais nenhum: o lock vale da decisão
/// de bloquear até a escolha, sem soltar e pegar de novo, e é solto antes da troca.
fn switch_from(cpu: &PerCpu, mut queue: IrqGuard<'static, Queue>, now_ns: u64) {
    check_locks_held(1);
    let from = cpu.current.load(Ordering::Relaxed) as usize;
    let outgoing = slot(from);
    let since = outgoing.since_ns.load(Ordering::Relaxed);
    bump(&outgoing.cpu_ns, now_ns.saturating_sub(since));
    if outgoing.state() == State::Running {
        outgoing.set_state(State::Ready);
    }
    let idle = queue.idle;
    let to = queue
        .policy()
        .pick_next(now_ns)
        .map_or(idle, |id| id.as_u32() as usize);
    let incoming = slot(to);
    incoming.set_state(State::Running);
    incoming.since_ns.store(now_ns, Ordering::Relaxed);
    if to != from {
        incoming.on_cpu.store(true, Ordering::Release);
        bump(&incoming.switches, 1);
        bump(&cpu.stats.switches, 1);
    }
    queue.rearm(now_ns);
    publish(cpu, &queue);
    if to == from {
        return;
    }
    cpu.current.store(slot_u32(to), Ordering::Relaxed);
    cpu.prev.store(slot_u32(from), Ordering::Relaxed);
    drop(queue);
    if let Some(area) = fpu_pointer(outgoing) {
        // SAFETY: a área é da tarefa que sai, viva, e tem o tamanho que `fpu_init` mediu.
        unsafe { platform::fpu_save(area.as_ptr()) };
    }
    if let Some(area) = fpu_pointer(incoming) {
        // SAFETY: a área é da tarefa que entra, viva, e foi preparada ou salva antes.
        unsafe { platform::fpu_restore(area.as_ptr()) };
    }
    let (leaving, entering) = (
        outgoing.user_active.load(Ordering::Relaxed),
        incoming.user_active.load(Ordering::Relaxed),
    );
    let index = cpu.index();
    if leaving != entering {
        mark_active(entering, index, true);
    }
    platform::switch_space(
        incoming.user_root.load(Ordering::Relaxed),
        incoming.user_top.load(Ordering::Relaxed),
    );
    if leaving != entering {
        mark_active(leaving, index, false);
    }
    // SAFETY: `to` foi preparada por `start` ou guardada por uma troca anterior; a IRQ está
    // mascarada, nenhum lock está preso e nenhuma outra CPU a usa (`on_cpu`).
    unsafe { platform::switch(CONTEXTS.0[from].get(), CONTEXTS.0[to].get()) };
    finish_switch();
}

/// Sobe ou baixa o bit de `cpu` na máscara de CPUs do espaço em `mask` (zero: tarefa sem espaço).
/// O bit sobe antes de a raiz entrar nesta CPU e baixa depois de ela sair: quem invalida o TLB lê a
/// máscara e nunca perde uma CPU que ainda pode ter a tradução.
fn mark_active(mask: usize, cpu: usize, on: bool) {
    if mask == 0 {
        return;
    }
    // SAFETY: o endereço é o do `AtomicU32` de um `UserSpace` ligado por `bind_user`, que vive até
    // `unbind_user` e não se move enquanto a tarefa o roda.
    let mask = unsafe { &*core::ptr::with_exposed_provenance::<AtomicU32>(mask) };
    if on {
        mask.fetch_or(1 << cpu, Ordering::SeqCst);
    } else {
        mask.fetch_and(!(1 << cpu), Ordering::SeqCst);
    }
}

/// Depois de uma troca, já na pilha de quem entrou: libera a tarefa que saiu desta CPU para
/// qualquer outra CPU escolhê-la. O `Release` publica o contexto que a troca acabou de salvar.
fn finish_switch() {
    #[cfg(feature = "selftest")]
    crate::process::check_ac();
    let cpu = percpu::current();
    let prev = cpu.prev.load(Ordering::Relaxed);
    if prev != NO_TASK {
        cpu.prev.store(NO_TASK, Ordering::Relaxed);
        slot(prev as usize).on_cpu.store(false, Ordering::Release);
    }
}

/// A área de FPU de uma tarefa que a usa e que ainda tem de ser salva.
fn fpu_pointer(task: &Slot) -> Option<NonNull<u8>> {
    let area = task.fpu.load(Ordering::Relaxed);
    if area != 0 && task.state() != State::Exited {
        NonNull::new(core::ptr::with_exposed_provenance_mut(area))
    } else {
        None
    }
}

/// Cede a CPU a outra tarefa pronta, se houver.
pub fn yield_now() {
    let cpu = platform::cpu();
    let were_enabled = cpu.save_and_disable_interrupts();
    let now = now();
    let here = percpu::current();
    assert_task_context(here);
    let mut queue = here.queue.lock();
    if here.current.load(Ordering::Relaxed) as usize != queue.idle {
        queue.policy().yield_current(now);
    }
    switch_from(here, queue, now);
    cpu.restore_interrupts(were_enabled);
}

/// Dorme até o relógio (`now_ns`) chegar a `deadline_ns`.
pub fn sleep_until(deadline_ns: u64) {
    let cpu = platform::cpu();
    let were_enabled = cpu.save_and_disable_interrupts();
    let now = now();
    let here = percpu::current();
    assert_task_context(here);
    if now < deadline_ns {
        let mut queue = here.queue.lock();
        let current = here.current.load(Ordering::Relaxed) as usize;
        let task = slot(current);
        task.wake_at_ns.store(deadline_ns, Ordering::Relaxed);
        task.set_state(State::Sleeping);
        queue.next_wake_ns = queue.next_wake_ns.min(deadline_ns);
        queue.policy().sleep(id_of(current), now);
        switch_from(here, queue, now);
    }
    cpu.restore_interrupts(were_enabled);
}

/// Espera um `unpark`; volta já se ele chegou antes.
#[cfg(feature = "selftest")]
pub fn park() {
    let cpu = platform::cpu();
    let were_enabled = cpu.save_and_disable_interrupts();
    park_masked();
    cpu.restore_interrupts(were_enabled);
}

/// `park` para quem já decidiu com a interrupção mascarada, sem janela para perder o acordar.
pub fn park_masked() {
    let now = now();
    let here = percpu::current();
    assert_task_context(here);
    let mut queue = here.queue.lock();
    let current = here.current.load(Ordering::Relaxed) as usize;
    let task = slot(current);
    if task.permit.load(Ordering::Acquire) {
        task.permit.store(false, Ordering::Release);
        return;
    }
    task.set_state(State::Blocked);
    queue.policy().sleep(id_of(current), now);
    switch_from(here, queue, now);
}

/// Por que `wake_task` roda.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wake {
    /// Um `unpark`: acorda a bloqueada; para as outras vivas, deixa o aviso.
    Unpark,
    /// O prazo de quem dorme venceu.
    Timer,
    /// Um sinal de objeto mudou: acorda a bloqueada e a que dorme com prazo (`block_until`); para
    /// as outras vivas, deixa o aviso.
    Signal,
}

/// Acorda a tarefa dormindo ou bloqueada, na CPU menos carregada que a afinidade permite. Se
/// é outra fila, migra: destaca sob a de origem, solta, anexa sob a do destino. `true`: ela
/// entrou na fila desta CPU e deve preemptar a corrente (`notify`).
fn wake_task(id: usize, why: Wake, now: u64) -> bool {
    let task = slot(id);
    let (home, mut queue) = lock_owner(id);
    let wakes = match (task.state(), why) {
        (State::Blocked, Wake::Unpark) | (State::Blocked | State::Sleeping, Wake::Signal) => true,
        (State::Sleeping, Wake::Timer) => task.wake_at_ns.load(Ordering::Relaxed) <= now,
        (State::Free | State::New | State::Exited, _) => return false,
        (_, Wake::Unpark | Wake::Signal) => {
            task.permit.store(true, Ordering::Release);
            return false;
        }
        _ => false,
    };
    if !wakes {
        return false;
    }
    task.wake_at_ns.store(u64::MAX, Ordering::Relaxed);
    let target = wake_target(task, home);
    let home_cpu = percpu::get(home);
    if target == home {
        task.set_state(State::Ready);
        queue.policy().wake(id_of(id), now);
        publish(home_cpu, &queue);
        return notify(home_cpu, &queue, now);
    }
    let Some(moved) = queue.policy().detach(id_of(id)) else {
        return false;
    };
    task.set_state(State::Ready);
    task.cpu.store(slot_u32(target), Ordering::Release);
    publish(home_cpu, &queue);
    drop(queue);
    let destination = percpu::get(target);
    let mut queue = destination.queue.lock();
    if let Err(error) = queue.policy().add_with_lag(id_of(id), moved, now) {
        panic!("acordar perdeu a tarefa {id}: {error:?}");
    }
    publish(destination, &queue);
    notify(destination, &queue, now)
}

/// A CPU onde a tarefa acordada deve ficar: a dela mesma enquanto a troca que a tirou da CPU não
/// terminou (`on_cpu`), ou se está ociosa e a afinidade deixa; senão a menos carregada.
fn wake_target(task: &Slot, home: usize) -> usize {
    let mask = task.affinity.load(Ordering::Relaxed);
    if task.on_cpu.load(Ordering::Acquire) || (mask != 0 && mask.is_power_of_two()) {
        return home;
    }
    let home_idle = percpu::get(home).load.load(Ordering::Relaxed) == 0;
    if home_idle && task.may_run_on(home) {
        return home;
    }
    pick_cpu(task.affinity.load(Ordering::Relaxed), Some(home)).unwrap_or(home)
}

/// Pega o lock da fila que é dona da tarefa. A dona pode mudar entre ler `cpu` e pegar o
/// lock (migração em curso): confere de novo depois de pegar.
fn lock_owner(id: usize) -> (usize, IrqGuard<'static, Queue>) {
    let task = slot(id);
    loop {
        let owner = task.cpu.load(Ordering::Acquire) as usize;
        let guard = percpu::get(owner).queue.lock();
        if task.cpu.load(Ordering::Acquire) as usize == owner {
            return (owner, guard);
        }
        drop(guard);
    }
}

/// Acorda a tarefa de `id`, ou deixa o aviso para o próximo `park` dela.
/// Slot que não está vivo (`Free`, `New`, `Exited`) ignora o aviso.
/// Se a acordada deve rodar antes da corrente, a troca vem no fim da chamada (`preempt_now`).
pub fn unpark(id: TaskId) {
    let cpu = platform::cpu();
    let were_enabled = cpu.save_and_disable_interrupts();
    let now = now();
    if wake_task(id.as_u32() as usize, Wake::Unpark, now) {
        preempt_now(were_enabled, now);
    }
    cpu.restore_interrupts(were_enabled);
}

/// Troca já se a fila desta CPU pede. Não troca com a IRQ mascarada pelo chamador, dentro de IRQ
/// nem com `IrqLock` preso: o pedido vale na próxima IRQ (SDD 15, "Preempção"; DEBT-0065).
fn preempt_now(were_enabled: bool, now_ns: u64) {
    let here = percpu::current();
    if !were_enabled || here.in_irq() || here.locks_held() != 0 {
        return;
    }
    let queue = here.queue.lock();
    if queue.wants_resched() {
        switch_from(here, queue, now_ns);
    }
}

/// Encerra a tarefa atual; o slot e a pilha ficam para a próxima.
pub fn exit() -> ! {
    platform::cpu().save_and_disable_interrupts();
    let here = percpu::current();
    assert_task_context(here);
    let mut queue = here.queue.lock();
    let current = here.current.load(Ordering::Relaxed) as usize;
    slot(current).set_state(State::Exited);
    queue.policy().remove(id_of(current));
    switch_from(here, queue, now());
    unreachable!("tarefa encerrada voltou a rodar");
}

/// Ceder a CPU de dentro de IRQ trocaria a pilha da interrupção: recusa também em `release`.
fn assert_task_context(here: &PerCpu) {
    assert!(!here.in_irq(), "yield, sleep or park inside an interrupt");
    debug_assert!(is_active(here), "escalonador sem `init`");
}

/// Bloqueia a tarefa corrente até um `wake_waiters` ou até `deadline_ns` (`u64::MAX`: sem prazo).
/// Volta já se o aviso chegou antes; quem chama reavalia a condição, porque volta sem motivo certo.
pub fn block_until(deadline_ns: u64) {
    let cpu = platform::cpu();
    let were_enabled = cpu.save_and_disable_interrupts();
    let now = now();
    let here = percpu::current();
    assert_task_context(here);
    let mut queue = here.queue.lock();
    let current = here.current.load(Ordering::Relaxed) as usize;
    let task = slot(current);
    if task.permit.swap(false, Ordering::AcqRel) || now >= deadline_ns {
        drop(queue);
        cpu.restore_interrupts(were_enabled);
        return;
    }
    if deadline_ns == u64::MAX {
        task.set_state(State::Blocked);
    } else {
        task.wake_at_ns.store(deadline_ns, Ordering::Relaxed);
        task.set_state(State::Sleeping);
        queue.next_wake_ns = queue.next_wake_ns.min(deadline_ns);
    }
    queue.policy().sleep(id_of(current), now);
    switch_from(here, queue, now);
    cpu.restore_interrupts(were_enabled);
}

/// O bit da tarefa corrente na máscara de espera de um objeto (`wake_waiters`).
pub fn wait_bit() -> u64 {
    1 << current().as_u32()
}

/// Limpa o aviso pendente da tarefa corrente: vale antes de registrar uma espera nova.
pub fn clear_permit() {
    slot(current().as_u32() as usize)
        .permit
        .store(false, Ordering::Release);
}

/// Acorda as tarefas da máscara (um bit por slot). Pode rodar com `IrqLock` preso: só toma o lock
/// das filas, que fica abaixo de todos; a troca, se pedida, vem na próxima IRQ (DEBT-0065).
pub fn wake_waiters(mask: u64) {
    let cpu = platform::cpu();
    let were_enabled = cpu.save_and_disable_interrupts();
    let now = now();
    let mut preempt = false;
    for id in FIRST_TASK..MAX_TASKS {
        if mask >> id & 1 != 0 {
            preempt |= wake_task(id, Wake::Signal, now);
        }
    }
    if preempt {
        preempt_now(were_enabled, now);
    }
    cpu.restore_interrupts(were_enabled);
}

/// A tarefa que roda agora nesta CPU.
pub fn current() -> TaskId {
    TaskId::new(percpu::pinned(|cpu| cpu.current.load(Ordering::Relaxed)))
}

/// Liga a tarefa corrente à thread de usuário `process` (o `Arc::into_raw` de um `ThreadObj`) com
/// a raiz `root`: o espaço e a pilha de entrada desta CPU passam a ser os dele e as próximas trocas
/// os repõem. `active` é o endereço da máscara de CPUs do espaço, que vale até `unbind_user`. Deixa a
/// IRQ mascarada, para a entrada no usuário. Devolve o topo da pilha do kernel.
pub fn bind_user(root: u64, process: usize, active: usize) -> u64 {
    platform::cpu().save_and_disable_interrupts();
    let here = percpu::current();
    let current = slot(here.current.load(Ordering::Relaxed) as usize);
    let top = current
        .meta
        .lock()
        .stack
        .map_or(0, |stack| stack.top().as_u64());
    current.user_top.store(top, Ordering::Relaxed);
    current.user_root.store(root, Ordering::Relaxed);
    current.process.store(process, Ordering::Release);
    current.user_active.store(active, Ordering::Relaxed);
    mark_active(active, here.index(), true);
    platform::switch_space(root, top);
    top
}

/// A thread de usuário da tarefa corrente, ou `None` se ela não roda uma. Para o tratador de falta.
pub fn current_process() -> Option<usize> {
    let here = percpu::current();
    let current = slot(here.current.load(Ordering::Relaxed) as usize);
    Some(current.process.load(Ordering::Acquire)).filter(|&process| process != 0)
}

/// Desliga a tarefa corrente da thread: volta ao espaço do kernel nesta CPU e devolve o
/// ponteiro dela (ou `None` sem thread). Com a IRQ mascarada, antes de liberar qualquer tabela.
pub fn unbind_user() -> Option<usize> {
    let here = percpu::current();
    let current = slot(here.current.load(Ordering::Relaxed) as usize);
    platform::switch_space(0, 0);
    mark_active(
        current.user_active.swap(0, Ordering::Relaxed),
        here.index(),
        false,
    );
    current.user_root.store(0, Ordering::Relaxed);
    current.user_top.store(0, Ordering::Relaxed);
    let process = current.process.swap(0, Ordering::AcqRel);
    (process != 0).then_some(process)
}

/// Conta uma falta de página que só repetiu em `page`: `true` na segunda seguida na mesma página (a
/// tradução não se resolve: o processo morre em vez de prender a CPU). Outra página começa a conta.
pub fn note_retry(page: u64) -> bool {
    let here = percpu::current();
    let current = slot(here.current.load(Ordering::Relaxed) as usize);
    let mark = page | 1;
    current.retry_page.swap(mark, Ordering::Relaxed) == mark
}

/// A falta de página da tarefa corrente resolveu de verdade: a próxima repetição volta a valer.
pub fn clear_retry() {
    let here = percpu::current();
    slot(here.current.load(Ordering::Relaxed) as usize)
        .retry_page
        .store(0, Ordering::Relaxed);
}

/// Faz a tarefa `id` (um slot que o chamador sabe vivo) passar logo por um ponto em que o kernel a
/// vê: acorda a que espera e, se roda em outra CPU, manda o IPI de reescalonar. A morte de um
/// processo usa para parar as threads dele.
pub fn poke(id: usize) {
    let cpu = platform::cpu();
    let were_enabled = cpu.save_and_disable_interrupts();
    let now = now();
    let preempt = wake_task(id, Wake::Signal, now);
    let task = slot(id);
    let owner = task.cpu.load(Ordering::Acquire) as usize;
    if owner != percpu::current().index() && task.state() == State::Running {
        kick(percpu::get(owner));
    }
    if preempt {
        preempt_now(were_enabled, now);
    }
    cpu.restore_interrupts(were_enabled);
}

/// Guarda o pico da pilha da tarefa corrente, que a morte do processo ainda não repintou.
pub fn note_stack_peak() -> usize {
    let here = percpu::current();
    let current = slot(here.current.load(Ordering::Relaxed) as usize);
    let peak = current.meta.lock().stack.map_or(0, Stack::peak);
    TASK_PEAK.fetch_max(peak, Ordering::Relaxed);
    peak
}

/// O maior fundo da pilha de uma tarefa de usuário, medido na volta ao usuário.
pub fn note_user_depth(bytes: usize) {
    TASK_PEAK.fetch_max(bytes, Ordering::Relaxed);
}

pub fn set_current_name(name: &'static str) {
    slot(current().as_u32() as usize).meta.lock().name = name;
}

/// A tarefa espera em `park`, sem aviso pendente.
#[cfg(feature = "selftest")]
pub fn is_blocked(id: TaskId) -> bool {
    slot(id.as_u32() as usize).state() == State::Blocked
}

/// A tarefa terminou e o slot está livre para a próxima.
#[cfg(feature = "selftest")]
pub fn has_exited(id: TaskId) -> bool {
    slot(id.as_u32() as usize).state() == State::Exited
}

/// Quantas vezes a tarefa foi escolhida para rodar.
#[cfg(feature = "selftest")]
pub fn switches(id: TaskId) -> u64 {
    slot(id.as_u32() as usize).switches.load(Ordering::Relaxed)
}

/// Chamado da IRQ do timer: acorda quem dorme aqui com prazo vencido, cobra a corrente, puxa
/// trabalho se vale e arma o próximo disparo que a fila pede, ainda com o lock dela preso: quem
/// põe tarefa aqui lê o prazo armado sob o mesmo lock, e um prazo velho perderia o IPI.
pub fn timer_irq(now_ns: u64) {
    let here = percpu::current();
    let index = slot_u32(here.index());
    let mut next = u64::MAX;
    for id in FIRST_TASK..MAX_TASKS {
        let task = slot(id);
        if task.state() != State::Sleeping || task.cpu.load(Ordering::Relaxed) != index {
            continue;
        }
        let at = task.wake_at_ns.load(Ordering::Relaxed);
        if at <= now_ns {
            wake_task(id, Wake::Timer, now_ns);
        } else {
            next = next.min(at);
        }
    }
    balance(here, now_ns);
    let mut queue = here.queue.lock();
    queue.next_wake_ns = next;
    queue.policy().tick(now_ns);
    interrupts::arm_or_park(queue.next_timer_ns(now_ns), now_ns);
}

/// O pull periódico: a CPU com trabalho de sobra, a cada `BALANCE_NS`, acorda uma ociosa para
/// puxá-lo, e a que tem menos que as outras puxa direto. A ociosa roda `pull` ao acordar.
fn balance(here: &PerCpu, now: u64) {
    if now < here.next_balance_ns.load(Ordering::Relaxed) {
        return;
    }
    here.next_balance_ns
        .store(now + BALANCE_NS, Ordering::Relaxed);
    if here.load.load(Ordering::Relaxed) >= 2 {
        let active = ACTIVE.load(Ordering::Acquire);
        let idle = (0..MAX_CPUS).find(|&index| {
            index != here.index()
                && active >> index & 1 != 0
                && percpu::get(index).load.load(Ordering::Relaxed) == 0
        });
        if let Some(index) = idle {
            kick(percpu::get(index));
        }
    } else {
        pull(here.index(), 2);
    }
}

/// O escalonador desta CPU já tem a política ligada; antes disso o tique periódico manda.
pub fn is_ready() -> bool {
    percpu::pinned(is_active)
}

/// `is_ready` para quem já está com a interrupção mascarada.
fn is_active(cpu: &PerCpu) -> bool {
    ACTIVE.load(Ordering::Acquire) >> cpu.index() & 1 != 0
}

/// Chamado pelo `arch/` no fim de toda IRQ, depois do EOI, com a interrupção
/// mascarada e na pilha da tarefa interrompida: o quadro dela sobrevive à troca.
pub fn preempt() {
    let cpu = percpu::current();
    if is_active(cpu) && cpu.queue.lock().wants_resched() {
        schedule();
    }
}

/// Registra a tarefa atual como a que espera entrada, para a IRQ acordá-la, e dorme. A IRQ
/// pode ser de outra CPU: depois de registrar, `has_input` confere a fila de novo, porque um
/// byte que chegou entre a conferência de quem chama e o registro não teria quem o acordasse.
pub fn wait_for_input(has_input: impl Fn() -> bool) {
    let current = current().as_u32() as usize;
    INPUT_WAITER.store(current + 1, Ordering::SeqCst);
    fence(Ordering::SeqCst);
    if has_input() {
        INPUT_WAITER.store(0, Ordering::SeqCst);
        return;
    }
    park_masked();
}

/// Chamado da IRQ quando entrou byte na fila de entrada.
pub fn notify_input() {
    fence(Ordering::SeqCst);
    let waiter = INPUT_WAITER.swap(0, Ordering::SeqCst);
    if waiter != 0 {
        unpark(id_of(waiter - 1));
    }
}

/// O que `ps` mostra de uma tarefa.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub id: u32,
    pub name: &'static str,
    pub state: State,
    pub cpu: usize,
    pub cpu_ns: u64,
    pub switches: u64,
    pub simd: bool,
    /// Pico de uso da pilha e o tamanho dela, em bytes; zero sem pilha medida.
    pub stack_peak: usize,
    pub stack_bytes: usize,
}

/// Entrega cada tarefa viva a `visit`, slot a slot e fora de qualquer lock; sem array, que
/// em `release` ocupava muita pilha do shell.
pub fn for_each_snapshot(visit: &mut dyn FnMut(Snapshot)) {
    let now = now();
    let active = ACTIVE.load(Ordering::Acquire);
    for (id, task) in TASKS.iter().enumerate() {
        let state = task.state();
        let offline_idle = id < FIRST_TASK && active >> id & 1 == 0;
        if matches!(state, State::Free | State::Exited | State::New) || offline_idle {
            continue;
        }
        let running = if state == State::Running {
            now.saturating_sub(task.since_ns.load(Ordering::Relaxed))
        } else {
            0
        };
        let (name, stack) = {
            let meta = task.meta.lock();
            (meta.name, meta.stack)
        };
        visit(Snapshot {
            id: slot_u32(id),
            name,
            state,
            cpu: task.cpu.load(Ordering::Relaxed) as usize,
            cpu_ns: task.cpu_ns.load(Ordering::Relaxed) + running,
            switches: task.switches.load(Ordering::Relaxed),
            simd: task.fpu.load(Ordering::Relaxed) != 0,
            stack_peak: stack.map_or(0, Stack::peak),
            stack_bytes: stack.map_or(0, Stack::bytes),
        });
    }
}

/// O que `ps` e `bench` mostram de uma CPU.
#[derive(Clone, Copy, Debug)]
pub struct CpuSnapshot {
    pub index: usize,
    pub load: u32,
    pub idle_ns: u64,
    pub switches: u64,
    pub ipis_sent: u64,
    pub ipis_received: u64,
    pub pulled: u64,
}

/// Retrato de cada CPU ativa; o tempo ocioso inclui o que a ociosa acumula agora.
pub fn cpu_snapshots() -> [Option<CpuSnapshot>; MAX_CPUS] {
    let mut out = [None; MAX_CPUS];
    let now = now();
    let active = ACTIVE.load(Ordering::Acquire);
    for (index, entry) in out.iter_mut().enumerate() {
        if active >> index & 1 == 0 {
            continue;
        }
        let cpu = percpu::get(index);
        let idle = slot(index);
        let running = if idle.state() == State::Running {
            now.saturating_sub(idle.since_ns.load(Ordering::Relaxed))
        } else {
            0
        };
        *entry = Some(CpuSnapshot {
            index,
            load: cpu.load.load(Ordering::Relaxed),
            idle_ns: idle.cpu_ns.load(Ordering::Relaxed) + running,
            switches: cpu.stats.switches.load(Ordering::Relaxed),
            ipis_sent: cpu.stats.ipis_sent.load(Ordering::Relaxed),
            ipis_received: cpu.stats.ipis_received.load(Ordering::Relaxed),
            pulled: cpu.stats.pulled.load(Ordering::Relaxed),
        });
    }
    out
}

/// Disputas pelos locks das filas, de todas as CPUs, desde o boot.
pub fn queue_contentions() -> u32 {
    (0..MAX_CPUS)
        .map(|index| percpu::get(index).queue.contentions())
        .sum()
}

/// CPUs ativas no escalonador, um bit por CPU.
pub fn active_cpus() -> u32 {
    ACTIVE.load(Ordering::Acquire)
}

#[cfg(feature = "selftest")]
mod selftest;
#[cfg(feature = "selftest")]
pub use selftest::log_stacks;
