//! Processos de usuário (fatia 4a do SDD 17): um espaço de endereçamento e uma thread, sem objetos
//! nem handles. Entra em anel 3 ou EL0, morre por `process_exit` ou por falha da CPU, e o kernel
//! segue. O programa de teste embutido só existe no modo de teste; na 4b ele dá lugar ao `init`.
#![cfg_attr(
    not(feature = "selftest"),
    allow(
        dead_code,
        reason = "o init da 4b cria o processo; até lá só o modo de teste"
    )
)]

use alloc::boxed::Box;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use rustos_hal::abi::{
    ProcessEvent, USER_IMAGE_BASE, USER_TOP, event_kind, fault_access, fault_kind,
};
use rustos_hal::{Cpu, Fault, FaultKind, PAGE_SIZE, PageFlags};
use rustos_sched::SchedParams;

use crate::frames::Frames;
use crate::platform;
use crate::sync::IrqLock;
use crate::task::{self, Spawn, TaskError};
use crate::uspace::UserSpace;

/// A pilha inicial do usuário: 16 páginas (64 KiB) terminando aqui, com as páginas de cima e de
/// baixo sem mapeamento (guardas). Sem ASLR na 4a.
const STACK_TOP: u64 = 0x7000_0000_0000;
const STACK_PAGES: u64 = 16;

#[derive(Debug)]
pub enum ProcessError {
    Map(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] rustos_hal::MapError),
    Task(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] TaskError),
}

/// Um processo: o espaço, e onde a primeira thread começa.
#[derive(Debug)]
pub struct Process {
    pid: u32,
    space: UserSpace,
    entry: u64,
    sp: u64,
    arg0: u64,
}

/// O que um programa plano precisa para virar processo: os bytes de código (mapeados em
/// `USER_IMAGE_BASE`), o deslocamento da entrada e o primeiro argumento.
#[derive(Clone, Copy, Debug)]
pub struct Program<'a> {
    pub code: &'a [u8],
    pub entry_offset: usize,
    pub arg0: u64,
}

/// O maior fundo da pilha do kernel que uma falha de usuário alcançou, em bytes.
static FAULT_PEAK: AtomicUsize = AtomicUsize::new(0);

static NEXT_PID: AtomicU32 = AtomicU32::new(1);
/// O último evento de fim de processo, para o modo de teste conferir.
static LAST_EVENT: IrqLock<Option<(u32, ProcessEvent)>> = IrqLock::new(None);
/// A primeira syscall do boot já foi vista: o usuário entrou e voltou.
static FIRST_SYSCALL: AtomicBool = AtomicBool::new(false);

impl Process {
    /// Mapeia o código (só leitura e execução) e a pilha (leitura e escrita), sem execução.
    fn load(
        frames: Frames,
        direct_map_offset: u64,
        program: Program<'_>,
    ) -> Result<Self, ProcessError> {
        // SAFETY: o mapa direto cobre todo frame do alocador, e o kernel já roda nas próprias tabelas.
        let mut space =
            unsafe { UserSpace::new(frames, direct_map_offset) }.map_err(ProcessError::Map)?;
        let code_pages = (program.code.len() as u64).div_ceil(PAGE_SIZE).max(1);
        let mapped = space
            .map(
                USER_IMAGE_BASE,
                program.code,
                code_pages,
                PageFlags::USER_CODE,
            )
            .and_then(|()| {
                let base = STACK_TOP - STACK_PAGES * PAGE_SIZE;
                space.map(base, &[], STACK_PAGES, PageFlags::USER_DATA)
            });
        if let Err(error) = mapped {
            space.destroy();
            return Err(ProcessError::Map(error));
        }
        if !space.kernel_half_in_sync() {
            space.destroy();
            return Err(ProcessError::Map(rustos_hal::MapError::Unsupported(
                "kernel half out of sync",
            )));
        }
        Ok(Self {
            pid: NEXT_PID.fetch_add(1, Ordering::Relaxed),
            space,
            entry: USER_IMAGE_BASE + program.entry_offset as u64,
            sp: STACK_TOP,
            arg0: program.arg0,
        })
    }
}

/// Cria o processo e a tarefa que o roda. Devolve a tarefa e o número do processo.
pub fn spawn(
    frames: Frames,
    direct_map_offset: u64,
    program: Program<'_>,
    affinity: u32,
) -> Result<(rustos_sched::TaskId, u32), ProcessError> {
    let process = Process::load(frames, direct_map_offset, program)?;
    let pid = process.pid;
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
        Ok(id) => Ok((id, pid)),
        Err(error) => {
            // SAFETY: `raw` veio de `Box::into_raw` acima e a tarefa não chegou a existir.
            let process = unsafe { Box::from_raw(raw) };
            process.space.destroy();
            Err(ProcessError::Task(error))
        }
    }
}

/// A primeira instrução da tarefa do processo: liga o espaço nesta CPU e entra no usuário.
fn process_main(arg: usize) {
    let process = ptr::with_exposed_provenance_mut::<Process>(arg);
    // SAFETY: `arg` é o `Box::into_raw` de `spawn`, entregue só a esta tarefa.
    let (root, entry, sp, arg0) = unsafe {
        let p = &*process;
        (p.space.root().as_u64(), p.entry, p.sp, p.arg0)
    };
    let top = task::bind_user(root, arg);
    // SAFETY: `bind_user` mascarou a IRQ e já pôs o espaço e a pilha de entrada desta CPU; `entry` e
    // `sp` vêm de `load`, dentro de `[USER_MIN, USER_TOP)`, `sp` alinhado a 16.
    unsafe { platform::enter_user(entry, sp, arg0, 0, top) }
}

/// Ocorreu a primeira chamada de usuário do boot: diz, uma vez, que o anel 3 ou EL0 ficou de pé.
pub fn note_first_syscall() {
    if !FIRST_SYSCALL.swap(true, Ordering::Relaxed) {
        kprintln!("[rustos] user: entered ring3/el0");
    }
}

/// O fim do processo por `process_exit`. Nunca volta.
pub fn exit(code: i64) -> ! {
    terminate(ProcessEvent {
        kind: event_kind::EXIT,
        code,
        ..ProcessEvent::default()
    })
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
    terminate(ProcessEvent {
        kind: event_kind::FAULT,
        fault_kind: kind,
        access,
        pc: fault.pc.as_u64().min(USER_TOP),
        sp: fault.user_sp,
        addr: if addr < USER_TOP { addr } else { 0 },
        isa_code: fault.code,
        ..ProcessEvent::default()
    })
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
fn terminate(event: ProcessEvent) -> ! {
    platform::cpu().save_and_disable_interrupts();
    let Some(raw) = task::unbind_user() else {
        panic!("falha de usuário sem processo na tarefa");
    };
    // SAFETY: `raw` é o `Box::into_raw` de `spawn`, guardado na tarefa e tirado dela agora.
    let process = unsafe { Box::from_raw(ptr::with_exposed_provenance_mut::<Process>(raw)) };
    let pid = process.pid;
    let cpu = crate::percpu::pinned(crate::percpu::PerCpu::index);
    process.space.destroy();
    if event.kind == event_kind::EXIT {
        kprintln!(
            "[rustos] user: pid {pid} exited (code {}, cpu {cpu})",
            event.code
        );
    } else {
        log_fault(pid, &event);
        kprintln!("[rustos] user: fault contained (pid {pid})");
    }
    *LAST_EVENT.lock() = Some((pid, event));
    if crate::scrub::measuring() {
        let peak = task::note_stack_peak();
        if event.kind == event_kind::FAULT {
            FAULT_PEAK.fetch_max(peak, Ordering::Relaxed);
        }
    }
    task::exit()
}

fn log_fault(pid: u32, event: &ProcessEvent) {
    let name = fault_name(event.fault_kind);
    if event.fault_kind == fault_kind::PAGE_FAULT && event.addr == 0 {
        kprintln!("[rustos] user: pid {pid} stopped by a {name} outside the user half");
    } else if event.fault_kind == fault_kind::PAGE_FAULT {
        kprintln!(
            "[rustos] user: pid {pid} stopped by a {name} at {:#x} (pc {:#x})",
            event.addr,
            event.pc
        );
    } else {
        kprintln!(
            "[rustos] user: pid {pid} stopped by {name} (pc {:#x}, isa code {:#x})",
            event.pc,
            event.isa_code
        );
    }
}

/// O maior fundo da pilha do kernel numa falha de usuário.
pub fn fault_peak() -> usize {
    FAULT_PEAK.load(Ordering::Relaxed)
}

/// O último evento de fim de processo e de que processo foi.
pub fn last_event() -> Option<(u32, ProcessEvent)> {
    *LAST_EVENT.lock()
}

#[cfg(feature = "selftest")]
mod selftest;
#[cfg(feature = "selftest")]
pub use selftest::{check_ac, on_nop, self_test};
