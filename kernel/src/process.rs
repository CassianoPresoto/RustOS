//! Processos de usuário (SDD 17, fatias 4a a 4c): um espaço de endereçamento com a tabela de handles,
//! a memória (`mem`) e uma ou mais threads. Nasce de um ELF do initramfs (o `init`) ou de
//! `process_create`, entra em anel 3 ou EL0, morre por `process_exit`, por falha da CPU, por
//! `process_kill` ou com a última thread. O programa plano de teste só existe no modo de teste.

use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
use core::ptr;
#[cfg(feature = "selftest")]
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering, fence};

use rustos_hal::abi::{
    self, Handle, PROCESS_KILLED_CODE, ProcessEvent, Status, USER_TOP, event_kind, fault_access,
    fault_kind,
};
use rustos_hal::{Cpu, Fault, FaultKind, PhysAddr};
use rustos_mm::vmar::Access;
use rustos_object::{HandleCount, Reservation, Transit, channel_write};
use rustos_sched::SchedParams;
use rustos_shell::TaskName;

use crate::frames::Frames;
use crate::loader::{self, LoadError, Loaded};
use crate::object::{ByteCharge, Bytes, Lifecycle, Obj, Quota, Table, VecSlots, Work};
use crate::sync::IrqLock;
use crate::task::{self, MAX_TASKS, Spawn, TaskError};
use crate::uspace::{SpaceStats, Unresolved, UserSpace};
use crate::{platform, tlb};

pub use mem::{MapRequest, Mem};

/// O que um processo custa ao criador, em bytes de heap do kernel: o objeto, a árvore de regiões,
/// os slots da tabela e o que o espaço guarda (as tabelas de página são frames, e entram na
/// promessa de frames, não aqui).
pub const PROCESS_BYTES: usize = 8 * 1024;
/// O que uma thread custa ao criador: o objeto, a área de FPU/SIMD (até uns 2,7 KiB) e o início.
pub const THREAD_BYTES: usize = 4 * 1024;

#[derive(Debug)]
pub enum ProcessError {
    Load(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] LoadError),
    Task(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] TaskError),
    Handles(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] Status),
    /// Os argumentos passam dos tetos de `abi::args_fit`: nenhum processo nasce.
    Arguments,
}

impl<E: Into<LoadError>> From<E> for ProcessError {
    fn from(error: E) -> Self {
        Self::Load(error.into())
    }
}

/// O que `thread_create` e `process_create` precisam do sistema e o `Process` não guarda: o alocador
/// de frames, o deslocamento do mapa direto e onde está o initramfs.
#[derive(Clone, Copy, Debug)]
struct System {
    frames: Frames,
    direct_map_offset: u64,
    initramfs: Option<(PhysAddr, u64)>,
}

static SYSTEM: IrqLock<Option<System>> = IrqLock::new(None);

/// Entrega ao módulo o que `process_create` usa. Uma vez, depois de `task::init`.
pub fn install(frames: Frames, direct_map_offset: u64) {
    *SYSTEM.lock() = Some(System {
        frames,
        direct_map_offset,
        initramfs: None,
    });
}

/// Diz onde o initramfs mora na memória física (início e bytes), para o `init` o receber como VMO.
pub fn set_initramfs(range: Option<(PhysAddr, u64)>) {
    if let Some(system) = SYSTEM.lock().as_mut() {
        system.initramfs = range;
    }
}

fn system() -> Option<System> {
    *SYSTEM.lock()
}

/// O alocador de frames, para quem cria objeto que os guarda (o VMO).
pub fn frames() -> Option<Frames> {
    system().map(|system| system.frames)
}

/// Onde um processo está na vida.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Sem thread ainda: `thread_create` o inicia.
    Created,
    Running,
    /// Morrendo: nenhuma thread nova; as que existem saem na próxima volta ao usuário.
    Dying,
    Dead,
}

#[derive(Debug)]
struct State {
    phase: Phase,
    /// Threads vivas, contadas de `thread_create` até a saída.
    threads: u32,
    /// Um bit por slot de tarefa das threads que já rodam (`MAX_TASKS <= 64`): quem mata acorda.
    mask: u64,
}

/// Um processo: o objeto que os handles apontam, a tabela, a memória e o estado de vida. As threads
/// seguram um `Arc` cada.
#[derive(Debug)]
pub struct Process {
    life: Lifecycle,
    pid: u32,
    mem: IrqLock<Option<Mem>>,
    table: IrqLock<Table>,
    /// O que o processo cria no kernel: pontas, mensagens, VMOs, filhos e threads.
    quota: Arc<Quota>,
    /// O que o criador paga por este objeto, devolvido quando ele some.
    #[allow(dead_code, reason = "só o `Drop` da cobrança importa")]
    cost: Option<ByteCharge>,
    state: IrqLock<State>,
    dying: AtomicBool,
    /// A última falta de página não se resolveu por falta de frame.
    out_of_memory: AtomicBool,
    /// A falha foi um acesso à metade do kernel (a frase da morte diz só isso).
    kernel_half: AtomicBool,
    /// O evento do fim: o primeiro que chega vale.
    event: IrqLock<Option<ProcessEvent>>,
    /// A ponta de eventos (o `events` de `process_create`), guardada numa tabela de uma posição:
    /// `channel_write` escreve por ela e `event_handle` diz qual é o handle (0, nenhum).
    events: IrqLock<Table>,
    event_handle: AtomicU64,
    /// O programa que o processo roda, como o `ps` o mostra; `user` sem nome dado.
    name: TaskName,
}

/// Uma thread como objeto: o que se espera (`TERMINATED`) e o processo dela.
#[derive(Debug)]
pub struct ThreadObj {
    life: Lifecycle,
    process: Arc<Process>,
    #[allow(dead_code, reason = "só o `Drop` da cobrança importa")]
    cost: Option<ByteCharge>,
}

/// O VMAR raiz de um processo como objeto: o teto de permissão é o dos direitos do handle.
#[derive(Debug)]
pub struct VmarObj {
    handles: HandleCount,
    process: Arc<Process>,
}

impl VmarObj {
    pub fn new(process: &Arc<Process>) -> Arc<Self> {
        Arc::new(Self {
            handles: HandleCount::new(0),
            process: Arc::clone(process),
        })
    }

    pub const fn handles(&self) -> &HandleCount {
        &self.handles
    }

    /// O processo dono do espaço.
    pub const fn process(&self) -> &Arc<Process> {
        &self.process
    }
}

impl ThreadObj {
    pub const fn lifecycle(&self) -> &Lifecycle {
        &self.life
    }
}

/// De onde vem o código do processo: um ELF do initramfs ou, no modo de teste, um programa plano.
#[derive(Clone, Copy, Debug)]
pub enum Source<'a> {
    Elf(&'a [u8]),
    #[cfg(feature = "selftest")]
    Flat(Program<'a>),
}

/// O que um programa plano precisa para virar processo: os bytes de código (mapeados em
/// `USER_IMAGE_BASE`), o deslocamento da entrada e o primeiro argumento.
#[cfg(feature = "selftest")]
#[derive(Clone, Copy, Debug)]
pub struct Program<'a> {
    pub code: &'a [u8],
    pub entry_offset: usize,
    pub arg0: u64,
}

/// O que `spawn` entrega: a tarefa, o número do processo, o que a carga custou e o processo, que quem
/// lança segura para esperá-lo ou matá-lo (as threads seguram as delas).
#[derive(Debug)]
pub struct Spawned {
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "lido só pelo modo de teste")
    )]
    pub task: rustos_sched::TaskId,
    pub pid: u32,
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "lido só pelo modo de teste")
    )]
    pub segments: usize,
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "lido só pelo modo de teste")
    )]
    pub stats: SpaceStats,
    pub process: Arc<Process>,
}

/// Os processos vivos por pid, para `kill` achar um. Uma posição por tarefa: um processo vivo tem
/// pelo menos uma thread, e com a tabela cheia o processo só fica sem `kill`.
type Registered = Option<(u32, Weak<Process>)>;
static PROCESSES: IrqLock<[Registered; MAX_TASKS]> = IrqLock::new([const { None }; MAX_TASKS]);

/// Põe `process` na tabela, numa posição vazia ou de um processo que já não existe. Tabela cheia: o
/// processo fica sem `kill`, sem pânico. O `Weak` que sai é solto fora do lock.
fn register(pid: u32, process: &Arc<Process>) {
    let displaced = {
        let mut table = PROCESSES.lock();
        let Some(slot) = table.iter_mut().find(|slot| {
            slot.as_ref()
                .is_none_or(|(_, weak)| weak.strong_count() == 0)
        }) else {
            return;
        };
        slot.replace((pid, Arc::downgrade(process)))
    };
    drop(displaced);
}

/// Tira `pid` da tabela na morte: o processo morto não fica achável e o `Weak` não segura a memória
/// do objeto até a posição ser reusada. O `Weak` que sai é solto fora do lock.
fn forget(pid: u32) {
    let displaced = {
        let mut table = PROCESSES.lock();
        table
            .iter_mut()
            .find(|slot| slot.as_ref().is_some_and(|(owner, _)| *owner == pid))
            .and_then(Option::take)
    };
    drop(displaced);
}

/// O processo vivo de `pid`, se a tabela ainda o tem.
pub fn find(pid: u32) -> Option<Arc<Process>> {
    let table = PROCESSES.lock();
    table
        .iter()
        .flatten()
        .find(|(owner, _)| *owner == pid)?
        .1
        .upgrade()
}

/// O maior fundo da pilha do kernel que uma falha de usuário alcançou, em bytes.
#[cfg(feature = "selftest")]
static FAULT_PEAK: AtomicUsize = AtomicUsize::new(0);

/// O nome da tarefa de um processo criado sem nome.
const DEFAULT_NAME: &str = "user";

static NEXT_PID: AtomicU32 = AtomicU32::new(1);
/// O maior número de CPUs que rodavam um processo no instante em que `process_kill` o parou.
#[cfg(feature = "selftest")]
static KILL_CPUS: AtomicU32 = AtomicU32::new(0);

/// O maior número de CPUs que rodavam um processo no instante de um `process_kill` (só o modo de teste).
#[cfg(feature = "selftest")]
pub fn kill_cpus_max() -> u32 {
    KILL_CPUS.load(Ordering::Relaxed)
}

/// O último evento de fim de processo, para o modo de teste conferir.
static LAST_EVENT: IrqLock<Option<(u32, ProcessEvent)>> = IrqLock::new(None);
/// Os frames do último processo que morreu, no instante da morte (só o modo de teste).
#[cfg(feature = "selftest")]
static LAST_STATS: IrqLock<Option<(u32, SpaceStats)>> = IrqLock::new(None);
/// A primeira syscall do boot já foi vista: o usuário entrou e voltou.
static FIRST_SYSCALL: AtomicBool = AtomicBool::new(false);
/// O contador de ciclos na primeira syscall do boot e na última entrada em usuário.
static FIRST_SYSCALL_CYCLES: AtomicU64 = AtomicU64::new(0);
static ENTRY_CYCLES: AtomicU64 = AtomicU64::new(0);

impl Process {
    /// Um processo novo sobre `mem`, sem thread e com a tabela vazia. `cost` é o que o criador paga.
    fn new(mem: Mem, cost: Option<ByteCharge>, name: TaskName) -> Arc<Self> {
        Arc::new(Self {
            life: Lifecycle::new(),
            pid: NEXT_PID.fetch_add(1, Ordering::Relaxed),
            mem: IrqLock::new(Some(mem)),
            table: IrqLock::new(Table::new(VecSlots::default())),
            quota: Arc::default(),
            cost,
            state: IrqLock::new(State {
                phase: Phase::Created,
                threads: 0,
                mask: 0,
            }),
            dying: AtomicBool::new(false),
            out_of_memory: AtomicBool::new(false),
            kernel_half: AtomicBool::new(false),
            event: IrqLock::new(None),
            events: IrqLock::new(Table::new(VecSlots::default())),
            event_handle: AtomicU64::new(Handle::INVALID.raw()),
            name,
        })
    }

    /// Um processo vazio para `process_create`: espaço sem imagem, árvore vazia e o `mmap_base`
    /// sorteado. A memória do espaço vem do alocador de frames; `cost` é a cobrança ao pai.
    pub fn create_empty(cost: ByteCharge, name: TaskName) -> Result<Arc<Self>, Status> {
        let system = system().ok_or(Status::NotSupported)?;
        // SAFETY: o mapa direto cobre todo frame do alocador, e o kernel já roda nas próprias tabelas.
        let space = unsafe { UserSpace::new(system.frames, system.direct_map_offset) }
            .map_err(|_| Status::NoMemory)?;
        if !space.kernel_half_in_sync() {
            space.destroy();
            return Err(Status::NoMemory);
        }
        let Ok(regions) = loader::new_tree() else {
            space.destroy();
            return Err(Status::NoMemory);
        };
        let mem = Mem::new(space, regions)?;
        Ok(Self::new(mem, Some(cost), name))
    }

    pub const fn lifecycle(&self) -> &Lifecycle {
        &self.life
    }

    /// A tabela de handles do processo.
    pub const fn table(&self) -> &IrqLock<Table> {
        &self.table
    }

    /// O que o processo cria no kernel: pontas, mensagens, VMOs, filhos e threads.
    pub const fn quota(&self) -> &Arc<Quota> {
        &self.quota
    }

    /// Reserva o lugar da ponta de eventos, antes de qualquer movimento: sem ele, nada se move.
    pub fn reserve_events(&self) -> Result<Reservation, Status> {
        self.events.lock().reserve()
    }

    /// Desfaz a reserva da ponta de eventos (o erro que vem depois dela).
    pub fn cancel_events(&self, slot: Reservation) {
        self.events.lock().cancel(slot);
    }

    /// Guarda a ponta de eventos no lugar reservado: conta em `handle_count` e o processo a escreve
    /// e fecha no fim (`report`).
    pub fn keep_events(&self, slot: Reservation, end: Transit<Obj>) {
        let handle = self.events.lock().commit_moved(slot, end);
        self.event_handle.store(handle.raw(), Ordering::Release);
    }

    /// Roda `f` com a memória do processo, esperando o *shootdown* das outras CPUs enquanto gira no
    /// lock. `BadState` se o processo já morreu.
    pub fn with_mem<R>(&self, f: impl FnOnce(&mut Mem) -> R) -> Result<R, Status> {
        let mut guard = self.mem.lock_serving(tlb::service);
        guard.as_mut().map(f).ok_or(Status::BadState)
    }

    /// Está morrendo: toda thread que passa por um ponto de volta ao usuário sai.
    pub fn is_dying(&self) -> bool {
        self.dying.load(Ordering::SeqCst)
    }

    /// Reserva o lugar de uma thread nova: `BadState` se o processo morre ou morreu.
    fn begin_thread(&self) -> Result<(), Status> {
        let mut state = self.state.lock();
        match state.phase {
            Phase::Created | Phase::Running => {
                state.phase = Phase::Running;
                state.threads += 1;
                Ok(())
            }
            Phase::Dying | Phase::Dead => Err(Status::BadState),
        }
    }

    /// Desfaz `begin_thread` de uma thread que não chegou a nascer.
    fn abort_thread(&self) {
        let mut state = self.state.lock();
        state.threads -= 1;
        if state.threads == 0 && state.phase == Phase::Running && state.mask == 0 {
            state.phase = Phase::Created;
        }
    }

    /// A thread da tarefa `bit` começou a rodar.
    fn thread_started(&self, bit: u64) {
        self.state.lock().mask |= bit;
    }

    /// A thread saiu da raiz do espaço. `true` se foi a última: quem a recebe desfaz o processo.
    fn thread_left(&self, bit: u64) -> bool {
        let mut state = self.state.lock();
        state.mask &= !bit;
        state.threads -= 1;
        if state.threads > 0 {
            return false;
        }
        if state.phase == Phase::Running {
            state.phase = Phase::Dying;
            self.dying.store(true, Ordering::SeqCst);
            let mut event = self.event.lock();
            event.get_or_insert(ProcessEvent {
                kind: event_kind::EXIT,
                ..ProcessEvent::default()
            });
        }
        state.phase == Phase::Dying
    }

    /// O processo começa a morrer com `event` (o primeiro vale) e as threads dos outros slots são
    /// paradas: acordadas, ou com o IPI se rodam em outra CPU. `None` se já morria; `Some(true)` se
    /// não tinha thread, lido sob o mesmo lock: só então quem chama desfaz (senão, a última thread).
    fn begin_death(&self, event: ProcessEvent, own_bit: u64) -> Option<bool> {
        let mut state = self.state.lock();
        if matches!(state.phase, Phase::Dying | Phase::Dead) {
            return None;
        }
        state.phase = Phase::Dying;
        self.dying.store(true, Ordering::SeqCst);
        *self.event.lock() = Some(event);
        fence(Ordering::SeqCst);
        let mask = state.mask & !own_bit;
        for id in (0..64).filter(|id| mask >> id & 1 != 0) {
            task::stop(id);
        }
        Some(state.threads == 0)
    }

    /// `process_kill`: o processo morre com o código de kill. Sem thread, quem chama o desfaz já; com
    /// threads, a última a sair o desfaz. `BadState` se já morria.
    pub fn kill(self: &Arc<Self>) -> Result<(), Status> {
        let event = ProcessEvent {
            kind: event_kind::EXIT,
            code: PROCESS_KILLED_CODE,
            ..ProcessEvent::default()
        };
        #[cfg(feature = "selftest")]
        if let Ok(active) = self.with_mem(|mem| mem.space.active_cpus()) {
            KILL_CPUS.fetch_max(active.count_ones(), Ordering::Relaxed);
        }
        let Some(idle) = self.begin_death(event, 0) else {
            return Err(Status::BadState);
        };
        if idle {
            self.teardown();
        }
        Ok(())
    }

    /// Desfaz o processo: nenhuma thread roda mais, então nenhuma CPU o ativa. Fecha os handles,
    /// devolve os frames, levanta `TERMINATED` e deixa o evento. Roda uma vez, na última thread a
    /// sair ou em quem matou um processo que nunca rodou.
    fn teardown(self: &Arc<Self>) {
        let cpu = crate::percpu::pinned(crate::percpu::PerCpu::index);
        let mem = self.mem.lock_serving(tlb::service).take();
        close_handles(&self.table);
        if let Some(mem) = mem {
            mem.wait_inactive();
            #[cfg(feature = "selftest")]
            {
                *LAST_STATS.lock() = Some((self.pid, mem.space.stats()));
            }
            mem.destroy();
        }
        let event = self.event.lock().take().unwrap_or_default();
        let no_memory = self.out_of_memory.load(Ordering::Relaxed);
        let kernel_half = self.kernel_half.load(Ordering::Relaxed);
        self.state.lock().phase = Phase::Dead;
        // O evento antes de `TERMINATED`: quem acorda pelo sinal já acha a mensagem na ponta.
        self.report(&event);
        self.life.terminate();
        let pid = self.pid;
        if event.kind == event_kind::EXIT && event.code == PROCESS_KILLED_CODE {
            kprintln_serial!("[rustos] user: pid {pid} killed (cpu {cpu})");
        } else if event.kind == event_kind::EXIT {
            kprintln_serial!(
                "[rustos] user: pid {pid} exited (code {}, cpu {cpu})",
                event.code
            );
        } else {
            log_fault(pid, &event, no_memory, kernel_half);
            kprintln_serial!("[rustos] user: fault contained (pid {pid})");
        }
        *LAST_EVENT.lock() = Some((pid, event));
        forget(pid);
        crate::programs::finished(pid, &event, no_memory, kernel_half);
    }

    /// Escreve o evento final na ponta de eventos e a fecha, nessa ordem: quem a guarda lê a
    /// mensagem e só depois vê `PEER_CLOSED`. Sem ponta, nada. Uma ponta cheia ou já fechada pelo
    /// pai perde a mensagem em silêncio: o processo morre do mesmo jeito.
    fn report(&self, event: &ProcessEvent) {
        let handle = Handle::from_raw(self.event_handle.load(Ordering::Acquire));
        if let Some(handle) = handle {
            let bytes = Bytes::uncharged(&wire(event).to_bytes());
            if let Some(bytes) = bytes {
                let _ = channel_write(&self.events, handle, bytes, &[], &mut Work::new());
            }
        }
        close_handles(&self.events);
    }
}

/// O evento como sai do kernel: só endereço de usuário, o que passa do topo da metade baixa vira 0.
fn wire(event: &ProcessEvent) -> ProcessEvent {
    ProcessEvent {
        pc: user_address(event.pc),
        sp: user_address(event.sp),
        addr: user_address(event.addr),
        ..*event
    }
}

/// Um endereço que sai para fora do kernel (evento, log): `0` se não está na metade de usuário.
const fn user_address(value: u64) -> u64 {
    if value < USER_TOP { value } else { 0 }
}

/// O processo que nunca rodou (ou nunca foi desfeito) e perdeu o último handle: desfaz-se aqui.
impl Drop for Process {
    fn drop(&mut self) {
        if self.state.lock().phase == Phase::Dead {
            return;
        }
        let mem = self.mem.lock_serving(tlb::service).take();
        close_handles(&self.table);
        close_handles(&self.events);
        if let Some(mem) = mem {
            mem.wait_inactive();
            mem.destroy();
        }
    }
}

/// Roda `f` com a thread de usuário da tarefa corrente; `None` numa tarefa só de kernel. `f` não pode
/// matar a thread: só `leave_current` a solta, e ela roda na própria tarefa, fora desta chamada.
fn current_thread<R>(f: impl FnOnce(&ThreadObj) -> R) -> Option<R> {
    let raw = task::current_process()?;
    // SAFETY: `raw` é o `Arc::into_raw` de `thread_main`, vivo até `leave_current` rodar nesta tarefa.
    let thread = unsafe { &*ptr::with_exposed_provenance::<ThreadObj>(raw) };
    Some(f(thread))
}

/// Roda `f` com o processo da tarefa corrente; `None` numa tarefa só de kernel.
pub fn with_current<R>(f: impl FnOnce(&Process) -> R) -> Option<R> {
    current_thread(|thread| f(&thread.process))
}

/// Esvazia a tabela e fecha cada handle (SDD 17, a morte do processo): os objetos recebem os
/// eventos, e uma ponta de canal que perde o último handle levanta `PEER_CLOSED` na outra.
fn close_handles(table: &IrqLock<Table>) {
    let mut work = Work::new();
    work.reserve(table.lock().len());
    table.lock().drain(&mut work);
    rustos_object::close_pending(&mut work);
}

/// O programa plano: o código em `USER_IMAGE_BASE`, só leitura e execução.
#[cfg(feature = "selftest")]
fn load_flat(
    space: &mut UserSpace,
    regions: &mut loader::RegionTree,
    program: Program<'_>,
) -> Result<Loaded, LoadError> {
    use rustos_hal::abi::USER_IMAGE_BASE;
    use rustos_hal::{PAGE_SIZE, PageFlags};
    use rustos_mm::vmar::{Attrs, Kind, Perms};

    let pages = (program.code.len() as u64).div_ceil(PAGE_SIZE).max(1);
    let perms = Perms::READ.union(Perms::EXECUTE);
    regions.reserve(
        USER_IMAGE_BASE,
        pages * PAGE_SIZE,
        Attrs::new(Kind::Image, perms),
    )?;
    space.map_at(
        USER_IMAGE_BASE,
        0,
        program.code,
        pages,
        PageFlags::USER_CODE,
    )?;
    Ok(Loaded {
        entry: USER_IMAGE_BASE + program.entry_offset as u64,
        segments: 1,
        phdr: None,
        phnum: 0,
    })
}

/// Como `spawn`, com o que o processo recebe além do básico.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// Devolve a ponta do kernel do canal de boot (só o modo de teste).
    pub keep_peer: bool,
    /// O initramfs vai ao programa como VMO físico, no canal de boot (só o `init`).
    pub initramfs: bool,
}

/// Cria o processo e a tarefa que o roda. Devolve a tarefa e o número do processo.
#[cfg_attr(
    not(feature = "selftest"),
    allow(
        dead_code,
        reason = "só o modo de teste cria sem nome; o resto usa `spawn_as`"
    )
)]
pub fn spawn(
    frames: Frames,
    direct_map_offset: u64,
    source: Source<'_>,
    affinity: u32,
) -> Result<Spawned, ProcessError> {
    spawn_with(
        frames,
        direct_map_offset,
        source,
        affinity,
        Options::default(),
        DEFAULT_NAME,
        &[],
    )
    .map(|(spawned, _)| spawned)
}

/// Como `spawn`, com o nome do programa que a tarefa mostra no `ps` (cortado ao tamanho do campo) e
/// o `argv` inteiro, com o nome do programa primeiro (vazio deixa `argc` em 0).
pub fn spawn_as(
    frames: Frames,
    direct_map_offset: u64,
    source: Source<'_>,
    affinity: u32,
    name: &str,
    arguments: &[&[u8]],
) -> Result<Spawned, ProcessError> {
    spawn_with(
        frames,
        direct_map_offset,
        source,
        affinity,
        Options::default(),
        name,
        arguments,
    )
    .map(|(spawned, _)| spawned)
}

#[cfg_attr(
    not(feature = "selftest"),
    allow(
        dead_code,
        reason = "só o modo de teste guarda a ponta do canal de boot"
    )
)]
/// Como `spawn`; com `keep_peer`, devolve também a ponta do kernel do canal de boot, que fica
/// aberta até quem a recebe fechá-la.
pub fn spawn_keeping(
    frames: Frames,
    direct_map_offset: u64,
    source: Source<'_>,
    affinity: u32,
    keep_peer: bool,
) -> Result<(Spawned, Option<boot::BootPeer>), ProcessError> {
    let options = Options {
        keep_peer,
        initramfs: false,
    };
    spawn_with(
        frames,
        direct_map_offset,
        source,
        affinity,
        options,
        DEFAULT_NAME,
        &[],
    )
}

/// Monta o espaço (a imagem de `source`, a pilha inicial com `args` e as regiões), o processo, o canal
/// de boot e a primeira thread. `name` é o que o `ps` mostra da tarefa (o ponto único de criação).
pub fn spawn_with(
    frames: Frames,
    direct_map_offset: u64,
    source: Source<'_>,
    affinity: u32,
    options: Options,
    name: &str,
    arguments: &[&[u8]],
) -> Result<(Spawned, Option<boot::BootPeer>), ProcessError> {
    if !abi::args_fit(arguments) {
        return Err(ProcessError::Arguments);
    }
    // SAFETY: o mapa direto cobre todo frame do alocador, e o kernel já roda nas próprias tabelas.
    let mut space = unsafe { UserSpace::new(frames, direct_map_offset) }?;
    let built = populate(&mut space, source, arguments);
    let (regions, loaded, sp, arg0) = match built {
        Ok(parts) => parts,
        Err(error) => {
            space.destroy();
            return Err(error);
        }
    };
    if !space.kernel_half_in_sync() {
        space.destroy();
        return Err(rustos_hal::MapError::Unsupported("kernel half out of sync").into());
    }
    let mem = Mem::new(space, regions).map_err(ProcessError::Handles)?;
    let stats = mem.space.stats();
    let process = Process::new(mem, None, TaskName::new(name));
    let (arg0, peer) =
        boot::handles_of(&process, source, arg0, options).map_err(ProcessError::Handles)?;
    let thread = ThreadObj::new(&process, None);
    let pid = process.pid;
    // Antes de a thread rodar: a morte, que tira o pid da tabela, nunca chega antes do registro.
    register(pid, &process);
    let task = start_thread(
        &process,
        thread,
        Entry {
            pc: loaded.entry,
            sp,
            arg0,
            arg1: 0,
        },
        affinity,
    )
    .map_err(|status| {
        forget(pid);
        // O número só vale a um processo que existiu: devolve-o se ninguém pegou o seguinte.
        let _ = NEXT_PID.compare_exchange(pid + 1, pid, Ordering::Relaxed, Ordering::Relaxed);
        ProcessError::Task(task_error(status))
    })?;
    Ok((
        Spawned {
            task,
            pid,
            segments: loaded.segments,
            stats,
            process,
        },
        peer,
    ))
}

fn task_error(status: Status) -> TaskError {
    if status == Status::NoMemory {
        TaskError::NoMemory
    } else {
        TaskError::NoSlot
    }
}

fn populate(
    space: &mut UserSpace,
    source: Source<'_>,
    arguments: &[&[u8]],
) -> Result<(loader::RegionTree, Loaded, u64, u64), ProcessError> {
    let mut regions = loader::new_tree()?;
    let (loaded, arg0) = match source {
        Source::Elf(file) => (loader::load_image(space, &mut regions, file)?, 0),
        #[cfg(feature = "selftest")]
        Source::Flat(program) => (load_flat(space, &mut regions, program)?, program.arg0),
    };
    let sp = loader::map_stack(space, &mut regions, &loaded, arguments)?;
    Ok((regions, loaded, sp, arg0))
}

impl ThreadObj {
    /// Uma thread nova de `process`; `cost` é a cobrança ao criador.
    pub fn new(process: &Arc<Process>, cost: Option<ByteCharge>) -> Arc<Self> {
        Arc::new(Self {
            life: Lifecycle::new(),
            process: Arc::clone(process),
            cost,
        })
    }
}

/// Onde a thread começa: entrada, pilha e os dois argumentos de `rdi`/`rsi` (`x0`/`x1`).
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub pc: u64,
    pub sp: u64,
    pub arg0: u64,
    pub arg1: u64,
}

/// O que a tarefa nova recebe: a thread e onde ela começa.
struct Start {
    thread: Arc<ThreadObj>,
    at: Entry,
}

/// Cria a tarefa de `thread` e a põe para rodar em `process`. `BadState` se o processo morre,
/// `NoMemory` sem slot de tarefa ou sem memória (nada muda).
pub fn start_thread(
    process: &Arc<Process>,
    thread: Arc<ThreadObj>,
    at: Entry,
    affinity: u32,
) -> Result<rustos_sched::TaskId, Status> {
    process.begin_thread()?;
    let raw = Box::into_raw(Box::new(Start { thread, at }));
    let started = task::spawn(Spawn {
        name: "user",
        entry: thread_main,
        arg: raw.expose_provenance(),
        simd: true,
        params: SchedParams::fair(0),
        affinity,
    });
    if let Ok(id) = started {
        return Ok(id);
    }
    // SAFETY: `raw` veio de `Box::into_raw` acima e a tarefa não chegou a existir.
    drop(unsafe { Box::from_raw(raw) });
    process.abort_thread();
    Err(Status::NoMemory)
}

/// A primeira instrução da tarefa de uma thread: liga o espaço nesta CPU e entra no usuário, a menos
/// que o processo já morra.
fn thread_main(arg: usize) {
    // SAFETY: `arg` é o `Box::into_raw` de `start_thread`, entregue só a esta tarefa. A caixa se
    // desfaz neste comando: `enter_user` não volta, e nenhum local com `Drop` pode sobrar até ele.
    let Start { thread, at } =
        *unsafe { Box::from_raw(ptr::with_exposed_provenance_mut::<Start>(arg)) };
    let process = Arc::clone(&thread.process);
    let pid = process.pid;
    let name = process.name;
    let (root, active) = process
        .with_mem(|mem| (mem.space.root().as_u64(), mem.space.active_addr()))
        .expect("o processo só desfaz com zero threads, e esta conta");
    process.thread_started(task::wait_bit());
    let raw = Arc::into_raw(thread).expose_provenance();
    drop(process);
    let top = task::bind_user(root, raw, active);
    task::set_pid(pid);
    task::set_user_name(name);
    if process_is_dying() {
        leave_current(None);
    }
    ENTRY_CYCLES.store(platform::cycles(), Ordering::Relaxed);
    // SAFETY: `bind_user` mascarou a IRQ e já pôs o espaço e a pilha de entrada desta CPU; `entry` e
    // `sp` foram conferidos pela quem criou a thread, dentro de `[USER_MIN, USER_TOP)`, `sp` alinhado a 16.
    unsafe { platform::enter_user(at.pc, at.sp, at.arg0, at.arg1, top) }
}

/// O processo da tarefa corrente já morre (a conferência lenta, só na entrada da thread).
fn process_is_dying() -> bool {
    current_thread(|thread| thread.process.is_dying()) == Some(true)
}

/// Se o processo da tarefa corrente morre, a thread sai: chamado em todo ponto de volta ao usuário
/// (fim de syscall, de falta resolvida, de IRQ e a entrada), com a IRQ mascarada.
#[allow(
    clippy::inline_always,
    reason = "no caminho da syscall nula: um quadro a mais passa da janela que o `arch/` repinta"
)]
#[inline(always)]
pub fn exit_if_dying() {
    if task::kill_requested() {
        leave_current(None);
    }
}

/// O fim de uma IRQ na tarefa de usuário: apaga a conta de repetição de falta e confere se o
/// processo morre.
pub fn after_irq() {
    if task::current_process().is_some() {
        task::clear_retry();
        exit_if_dying();
    }
}

/// Resolve a falta de página de usuário da tarefa corrente: região, permissão e guarda pelo `Vmar`, e
/// então página zero, frame novo, cópia da página zero ou frame de VMO. `false`: o processo morre
/// (inclusive a segunda repetição seguida na mesma página, que não se resolve).
pub fn resolve_user_fault(address: u64, write: bool, execute: bool) -> bool {
    let access = if execute {
        Access::Execute
    } else if write {
        Access::Write
    } else {
        Access::Read
    };
    current_thread(|thread| {
        let process = &thread.process;
        let resolved = process.with_mem(|mem| mem.resolve_fault(address, access));
        let Ok(resolved) = resolved else {
            return false;
        };
        process
            .out_of_memory
            .store(resolved == Err(Unresolved::OutOfMemory), Ordering::Relaxed);
        match resolved {
            Ok(crate::uspace::Resolved::Retried) => !task::note_retry(address & !4095),
            Ok(_) => {
                task::clear_retry();
                true
            }
            Err(_) => false,
        }
    })
    .unwrap_or(false)
}

/// A última falta de página da tarefa corrente falhou por falta de frame (e não por endereço ruim):
/// a cópia de uma syscall devolve `NoMemory` em vez de `OutOfRange`.
pub fn copy_ran_out_of_memory() -> bool {
    current_thread(|thread| thread.process.out_of_memory.load(Ordering::Relaxed)).unwrap_or(false)
}

/// Ocorreu a primeira chamada de usuário do boot: diz, uma vez, que o anel 3 ou EL0 ficou de pé.
pub fn note_first_syscall() {
    if !FIRST_SYSCALL.swap(true, Ordering::Relaxed) {
        FIRST_SYSCALL_CYCLES.store(platform::cycles(), Ordering::Relaxed);
        kprintln_serial!("[rustos] user: entered ring3/el0");
    }
}

/// O fim do processo por `process_exit`. Nunca volta.
pub fn exit(code: i64) -> ! {
    leave_current(Some(ProcessEvent {
        kind: event_kind::EXIT,
        code,
        ..ProcessEvent::default()
    }))
}

/// `thread_exit`: a thread sai; o processo segue com as outras e acaba com a última. Nunca volta.
pub fn thread_exit() -> ! {
    leave_current(None)
}

/// O fim do processo por uma falha da CPU: o processo morre e o kernel segue. Nenhum endereço do
/// kernel entra no evento nem no log (guidelines 09). Nunca volta.
pub fn user_fault(fault: &Fault) -> ! {
    let (kind, access, addr) = match fault.kind {
        FaultKind::PageFault {
            address,
            write,
            execute,
            present,
            ..
        } => (
            fault_kind::PAGE_FAULT,
            (u32::from(write) * fault_access::WRITE)
                | (u32::from(execute) * fault_access::EXECUTE)
                | (u32::from(present) * fault_access::PROTECTION),
            address.as_u64(),
        ),
        FaultKind::InvalidOpcode => (fault_kind::ILLEGAL_INSTRUCTION, 0, 0),
        FaultKind::Alignment => (fault_kind::UNALIGNED, 0, 0),
        FaultKind::DivideError => (fault_kind::ARITHMETIC, 0, 0),
        FaultKind::Breakpoint => (fault_kind::BREAKPOINT, 0, 0),
        _ => (fault_kind::OTHER, 0, 0),
    };
    // O evento nunca leva endereço do kernel (vira 0); a frase da falta diz só que foi na metade dele.
    let kernel_half = kind == fault_kind::PAGE_FAULT && addr >= USER_TOP;
    let _ = with_current(|process| process.kernel_half.store(kernel_half, Ordering::Relaxed));
    leave_current(Some(ProcessEvent {
        kind: event_kind::FAULT,
        fault_kind: kind,
        access,
        pc: user_address(fault.pc.as_u64()),
        sp: user_address(fault.user_sp),
        addr: user_address(addr),
        isa_code: fault.code,
        ..ProcessEvent::default()
    }))
}

fn fault_name(kind: u32) -> &'static str {
    match kind {
        fault_kind::PAGE_FAULT => "page fault",
        fault_kind::ILLEGAL_INSTRUCTION => "illegal instruction",
        fault_kind::UNALIGNED => "unaligned access",
        fault_kind::ARITHMETIC => "arithmetic fault",
        fault_kind::BREAKPOINT => "breakpoint",
        _ => "exception",
    }
}

/// A thread da tarefa corrente sai, na ordem do SDD 17: com `Some(event)` o processo inteiro começa a
/// morrer e as outras threads são paradas; sai da raiz do espaço antes de qualquer tabela ser liberada;
/// a última a sair desfaz o processo (handles, frames, `TERMINATED`); e só então a tarefa acaba.
#[cold]
#[inline(never)]
fn leave_current(event: Option<ProcessEvent>) -> ! {
    platform::cpu().save_and_disable_interrupts();
    let bit = task::wait_bit();
    let Some(raw) = task::current_process() else {
        panic!("falha de usuário sem thread na tarefa");
    };
    // SAFETY: `raw` é o `Arc::into_raw` de `thread_main`, vivo até esta saída.
    let process = Arc::clone(&unsafe { &*ptr::with_exposed_provenance::<ThreadObj>(raw) }.process);
    if let Some(event) = event {
        process.begin_death(event, bit);
    }
    let _ = task::unbind_user();
    // SAFETY: o ponteiro é o que `thread_main` guardou na tarefa e `unbind_user` acabou de tirar dela.
    let thread = unsafe { Arc::from_raw(ptr::with_exposed_provenance::<ThreadObj>(raw)) };
    thread.life.terminate();
    if process.thread_left(bit) {
        process.teardown();
        if crate::scrub::measuring() {
            let peak = task::note_stack_peak();
            #[cfg(feature = "selftest")]
            if LAST_EVENT
                .lock()
                .is_some_and(|(_, last)| last.kind == event_kind::FAULT)
            {
                FAULT_PEAK.fetch_max(peak, Ordering::Relaxed);
            }
            #[cfg(not(feature = "selftest"))]
            let _ = peak;
        }
    }
    drop(thread);
    drop(process);
    task::exit()
}

fn log_fault(pid: u32, event: &ProcessEvent, no_memory: bool, kernel_half: bool) {
    let name = fault_name(event.fault_kind);
    let reason = if no_memory { " (out of memory)" } else { "" };
    if kernel_half {
        kprintln_serial!("[rustos] user: pid {pid} stopped by a {name} in the kernel half{reason}");
    } else if event.fault_kind == fault_kind::PAGE_FAULT {
        kprintln_serial!(
            "[rustos] user: pid {pid} stopped by a {name} at {:#x} (pc {:#x}){reason}",
            event.addr,
            event.pc
        );
    } else {
        kprintln_serial!(
            "[rustos] user: pid {pid} stopped by {name} (pc {:#x}, isa code {:#x})",
            event.pc,
            event.isa_code
        );
    }
}

/// Os contadores de ciclo da última entrada em usuário e da primeira syscall do boot.
#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "lido só pelo modo de teste")
)]
pub fn entry_and_first_syscall_cycles() -> (u64, u64) {
    (
        ENTRY_CYCLES.load(Ordering::Relaxed),
        FIRST_SYSCALL_CYCLES.load(Ordering::Relaxed),
    )
}

/// O maior fundo da pilha do kernel numa falha de usuário.
#[cfg(feature = "selftest")]
pub fn fault_peak() -> usize {
    FAULT_PEAK.load(Ordering::Relaxed)
}

/// Os frames de dados e de tabelas do último processo que morreu, e de que processo foi.
#[cfg(feature = "selftest")]
pub fn last_stats() -> Option<(u32, SpaceStats)> {
    *LAST_STATS.lock()
}

/// O último evento de fim de processo e de que processo foi.
#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "lido só pelo modo de teste")
)]
pub fn last_event() -> Option<(u32, ProcessEvent)> {
    *LAST_EVENT.lock()
}

mod boot;
mod mem;
#[cfg(feature = "selftest")]
mod selftest;
#[cfg(feature = "selftest")]
pub use selftest::{check_ac, on_nop, self_test};
