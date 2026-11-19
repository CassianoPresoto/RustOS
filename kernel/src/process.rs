//! Processos de usuário (fatias 4a e 4b do SDD 17): um espaço de endereçamento e uma thread, sem
//! objetos nem handles. Nasce de um ELF do initramfs (o `init`), entra em anel 3 ou EL0, morre por
//! `process_exit` ou por falha da CPU, e o kernel segue. O programa plano de teste só existe no
//! modo de teste.

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::ptr;
#[cfg(feature = "selftest")]
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use rustos_hal::abi::{ProcessEvent, Status, USER_TOP, event_kind, fault_access, fault_kind};
use rustos_hal::{Cpu, Fault, FaultKind};
use rustos_mm::vmar::Access;
use rustos_sched::SchedParams;
use rustos_shell::TaskName;

use crate::frames::Frames;
use crate::loader::{self, LoadError, Loaded, RegionTree};
use crate::object::{ProcessObj, Quota, Table, VecSlots, Work};
use crate::platform;
use crate::sync::IrqLock;
use crate::task::{self, Spawn, TaskError};
use crate::uspace::{SpaceStats, Unresolved, UserSpace};

#[derive(Debug)]
pub enum ProcessError {
    Load(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] LoadError),
    Task(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] TaskError),
    Handles(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] Status),
}

impl<E: Into<LoadError>> From<E> for ProcessError {
    fn from(error: E) -> Self {
        Self::Load(error.into())
    }
}

/// Um processo: o espaço, a árvore de regiões e onde a primeira thread começa.
#[derive(Debug)]
pub struct Process {
    pid: u32,
    space: UserSpace,
    regions: RegionTree,
    entry: u64,
    sp: u64,
    /// O handle de boot de `rdi`/`x0`: a ponta do canal de boot no `init`.
    arg0: u64,
    /// O processo como objeto, que os handles de fora apontam e `TERMINATED` espera.
    object: Arc<ProcessObj>,
    /// A tabela de handles; só a tarefa do processo e a morte dele a tocam.
    table: IrqLock<Table>,
    /// Pontas de canal cobradas a este processo.
    channels: Arc<Quota>,
    /// A última falta de página não se resolveu por falta de frame.
    out_of_memory: bool,
    /// O programa que o processo roda, como o `ps` o mostra; `user` sem nome dado.
    name: TaskName,
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

/// O que `spawn` entrega: a tarefa, o número do processo e o que a carga custou.
#[derive(Clone, Copy, Debug)]
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
}

/// O maior fundo da pilha do kernel que uma falha de usuário alcançou, em bytes.
#[cfg(feature = "selftest")]
static FAULT_PEAK: AtomicUsize = AtomicUsize::new(0);

/// O nome da tarefa de um processo criado sem nome.
const DEFAULT_NAME: &str = "user";

static NEXT_PID: AtomicU32 = AtomicU32::new(1);
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
    /// Monta o espaço: a imagem (de `source`), a pilha inicial e a árvore de regiões.
    fn load(
        frames: Frames,
        direct_map_offset: u64,
        source: Source<'_>,
        keep_peer: bool,
    ) -> Result<(Self, Loaded, Option<boot::BootPeer>), ProcessError> {
        // SAFETY: o mapa direto cobre todo frame do alocador, e o kernel já roda nas próprias tabelas.
        let mut space = unsafe { UserSpace::new(frames, direct_map_offset) }?;
        let built = Self::populate(&mut space, source);
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
        let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
        let object = ProcessObj::new();
        let table = IrqLock::new(Table::new(VecSlots::default()));
        let (arg0, peer) = match boot::handles_of(&object, &table, source, arg0, keep_peer) {
            Ok(handed) => handed,
            Err(status) => {
                space.destroy();
                return Err(ProcessError::Handles(status));
            }
        };
        let process = Self {
            pid,
            space,
            regions,
            entry: loaded.entry,
            sp,
            arg0,
            object,
            table,
            channels: Arc::default(),
            out_of_memory: false,
            name: TaskName::new(DEFAULT_NAME),
        };
        Ok((process, loaded, peer))
    }

    fn populate(
        space: &mut UserSpace,
        source: Source<'_>,
    ) -> Result<(RegionTree, Loaded, u64, u64), ProcessError> {
        let mut regions = loader::new_tree()?;
        let (loaded, arg0) = match source {
            Source::Elf(file) => (loader::load_image(space, &mut regions, file)?, 0),
            #[cfg(feature = "selftest")]
            Source::Flat(program) => (load_flat(space, &mut regions, program)?, program.arg0),
        };
        let sp = loader::map_stack(space, &mut regions, &loaded)?;
        Ok((regions, loaded, sp, arg0))
    }
}

impl Process {
    /// A tabela de handles do processo.
    pub const fn table(&self) -> &IrqLock<Table> {
        &self.table
    }

    /// As pontas de canal cobradas ao processo.
    pub const fn channels(&self) -> &Arc<Quota> {
        &self.channels
    }
}

/// Roda `f` com o processo da tarefa corrente; `None` numa tarefa só de kernel. `f` não pode matar
/// o processo: só `terminate` o solta, e ele roda na própria tarefa, fora desta chamada.
pub fn with_current<R>(f: impl FnOnce(&Process) -> R) -> Option<R> {
    let raw = task::current_process()?;
    // SAFETY: `raw` é o `Box::into_raw` de `spawn`, vivo até `terminate` rodar nesta mesma tarefa.
    let process = unsafe { &*ptr::with_exposed_provenance::<Process>(raw) };
    Some(f(process))
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
    regions: &mut RegionTree,
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

/// Cria o processo e a tarefa que o roda. Devolve a tarefa e o número do processo.
#[cfg_attr(
    not(feature = "selftest"),
    allow(
        dead_code,
        reason = "usada pelo modo de teste e pela criação de processo por outro processo"
    )
)]
pub fn spawn(
    frames: Frames,
    direct_map_offset: u64,
    source: Source<'_>,
    affinity: u32,
) -> Result<Spawned, ProcessError> {
    spawn_keeping(frames, direct_map_offset, source, affinity, false).map(|(spawned, _)| spawned)
}

/// Como `spawn`, com o nome do programa que a tarefa mostra no `ps` (cortado ao tamanho do campo).
pub fn spawn_as(
    frames: Frames,
    direct_map_offset: u64,
    source: Source<'_>,
    affinity: u32,
    name: &str,
) -> Result<Spawned, ProcessError> {
    spawn_keeping_as(frames, direct_map_offset, source, affinity, false, name)
        .map(|(spawned, _)| spawned)
}

/// Como `spawn`; com `keep_peer`, devolve também a ponta do kernel do canal de boot, que fica
/// aberta até quem a recebe fechá-la.
#[cfg_attr(
    not(feature = "selftest"),
    allow(
        dead_code,
        reason = "usada pelo modo de teste e pela criação de processo por outro processo"
    )
)]
pub fn spawn_keeping(
    frames: Frames,
    direct_map_offset: u64,
    source: Source<'_>,
    affinity: u32,
    keep_peer: bool,
) -> Result<(Spawned, Option<boot::BootPeer>), ProcessError> {
    spawn_keeping_as(
        frames,
        direct_map_offset,
        source,
        affinity,
        keep_peer,
        DEFAULT_NAME,
    )
}

/// O ponto único que cria processo: `spawn` e `spawn_keeping` chegam aqui, e `name` vai ao `ps`.
pub fn spawn_keeping_as(
    frames: Frames,
    direct_map_offset: u64,
    source: Source<'_>,
    affinity: u32,
    keep_peer: bool,
    name: &str,
) -> Result<(Spawned, Option<boot::BootPeer>), ProcessError> {
    let (mut process, loaded, peer) = Process::load(frames, direct_map_offset, source, keep_peer)?;
    process.name = TaskName::new(name);
    let (pid, stats) = (process.pid, process.space.stats());
    let raw = Box::into_raw(Box::new(process));
    let started = task::spawn(Spawn {
        name: "user",
        entry: process_main,
        arg: raw.expose_provenance(),
        simd: true,
        params: SchedParams::fair(0),
        affinity,
    });
    match started {
        Ok(task) => Ok((
            Spawned {
                task,
                pid,
                segments: loaded.segments,
                stats,
            },
            peer,
        )),
        Err(error) => {
            // SAFETY: `raw` veio de `Box::into_raw` acima e a tarefa não chegou a existir.
            let process = unsafe { Box::from_raw(raw) };
            // O número só vale a um processo que existiu: devolve-o se ninguém pegou o seguinte.
            let _ = NEXT_PID.compare_exchange(pid + 1, pid, Ordering::Relaxed, Ordering::Relaxed);
            process.space.destroy();
            Err(ProcessError::Task(error))
        }
    }
}

/// A primeira instrução da tarefa do processo: liga o espaço nesta CPU e entra no usuário.
fn process_main(arg: usize) {
    let process = ptr::with_exposed_provenance_mut::<Process>(arg);
    // SAFETY: `arg` é o `Box::into_raw` de `spawn`, entregue só a esta tarefa.
    let (root, entry, sp, arg0, active, pid, name) = unsafe {
        let p = &*process;
        (
            p.space.root().as_u64(),
            p.entry,
            p.sp,
            p.arg0,
            p.space.active_addr(),
            p.pid,
            p.name,
        )
    };
    let top = task::bind_user(root, arg, active);
    task::set_pid(pid);
    task::set_user_name(name);
    ENTRY_CYCLES.store(platform::cycles(), Ordering::Relaxed);
    // SAFETY: `bind_user` mascarou a IRQ e já pôs o espaço e a pilha de entrada desta CPU; `entry` e
    // `sp` vêm de `load`, dentro de `[USER_MIN, USER_TOP)`, `sp` alinhado a 16.
    unsafe { platform::enter_user(entry, sp, arg0, 0, top) }
}

/// Resolve a falta de página de usuário da tarefa corrente: região, permissão e guarda pelo `Vmar`, e
/// então página zero, frame novo ou cópia da página zero. `false`: o processo morre.
pub fn resolve_user_fault(address: u64, write: bool, execute: bool) -> bool {
    let Some(raw) = task::current_process() else {
        return false;
    };
    // SAFETY: `raw` é o `Box::into_raw` de `spawn`, vivo até `terminate`; só a thread do processo
    // (a que falta) o toca, e roda mascarada (DEBT-0120).
    let process = unsafe { &mut *ptr::with_exposed_provenance_mut::<Process>(raw) };
    let access = if execute {
        Access::Execute
    } else if write {
        Access::Write
    } else {
        Access::Read
    };
    let resolved = process
        .space
        .resolve_fault(&process.regions, address, access);
    process.out_of_memory = resolved == Err(Unresolved::OutOfMemory);
    resolved.is_ok()
}

/// A última falta de página da tarefa corrente falhou por falta de frame (e não por endereço ruim):
/// a cópia de uma syscall devolve `NoMemory` em vez de `OutOfRange`.
pub fn copy_ran_out_of_memory() -> bool {
    task::current_process().is_some_and(|raw| {
        // SAFETY: como em `resolve_user_fault`.
        unsafe { &*ptr::with_exposed_provenance::<Process>(raw) }.out_of_memory
    })
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
    terminate(
        ProcessEvent {
            kind: event_kind::EXIT,
            code,
            ..ProcessEvent::default()
        },
        false,
    )
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
    terminate(
        ProcessEvent {
            kind: event_kind::FAULT,
            fault_kind: kind,
            access,
            pc: fault.pc.as_u64().min(USER_TOP),
            sp: fault.user_sp,
            addr: if addr < USER_TOP { addr } else { 0 },
            isa_code: fault.code,
            ..ProcessEvent::default()
        },
        kernel_half,
    )
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

/// Mata o processo da tarefa corrente, na ordem do SDD 17: sai da raiz do processo antes de
/// liberar qualquer tabela, devolve os frames zerados e só então encerra a tarefa.
fn terminate(event: ProcessEvent, kernel_half: bool) -> ! {
    // Tudo o que tem dono (o `Box<Process>`, o `Arc<ProcessObj>`, a cota) cai neste bloco: o que
    // sobrasse vivo até o `task::exit`, que não volta, nunca rodaria o `Drop` e vazaria heap.
    let (pid, no_memory, cpu) = {
        platform::cpu().save_and_disable_interrupts();
        let Some(raw) = task::unbind_user() else {
            panic!("falha de usuário sem processo na tarefa");
        };
        // SAFETY: `raw` é o `Box::into_raw` de `spawn`, guardado na tarefa e tirado dela agora.
        let process = unsafe { Box::from_raw(ptr::with_exposed_provenance_mut::<Process>(raw)) };
        let pid = process.pid;
        let no_memory = process.out_of_memory;
        let cpu = crate::percpu::pinned(crate::percpu::PerCpu::index);
        process.space.wait_inactive();
        let Process {
            space,
            regions,
            object,
            table,
            ..
        } = *process;
        close_handles(&table);
        drop(table);
        #[cfg(feature = "selftest")]
        {
            *LAST_STATS.lock() = Some((pid, space.stats()));
        }
        space.destroy();
        drop(regions);
        object.terminate();
        (pid, no_memory, cpu)
    };
    if event.kind == event_kind::EXIT {
        kprintln_serial!(
            "[rustos] user: pid {pid} exited (code {}, cpu {cpu})",
            event.code
        );
    } else {
        log_fault(pid, &event, no_memory, kernel_half);
        kprintln_serial!("[rustos] user: fault contained (pid {pid})");
    }
    *LAST_EVENT.lock() = Some((pid, event));
    crate::programs::finished(pid, &event, no_memory, kernel_half);
    if crate::scrub::measuring() {
        let peak = task::note_stack_peak();
        #[cfg(feature = "selftest")]
        if event.kind == event_kind::FAULT {
            FAULT_PEAK.fetch_max(peak, Ordering::Relaxed);
        }
        #[cfg(not(feature = "selftest"))]
        let _ = peak;
    }
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
#[cfg(feature = "selftest")]
mod selftest;
#[cfg(feature = "selftest")]
pub use selftest::{check_ac, on_nop, self_test};
