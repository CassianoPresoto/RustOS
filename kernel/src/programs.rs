//! O que o shell faz com programas do initramfs (SDD 17, "O M1"): listar os arquivos e rodar um
//! como processo, esperando ou não. O fim de cada processo chega por `finished`, numa tabela curta
//! que quem espera consulta; o processo morre na própria tarefa, sem ninguém o esperar por handle.

use rustos_cpio as cpio;
use rustos_hal::abi::{ProcessEvent, event_kind, fault_access, fault_kind};
use rustos_shell::{Access, KillCause, Listing, ProgramInfo, RunOutcome};

use crate::frames::Frames;
use crate::loader::{self, LoadError};
use crate::object::Waiters;
use crate::process::{self, ProcessError, Source};
use crate::sync::IrqLock;
use crate::task;
use crate::userout;

/// Quantos fins de processo a tabela lembra: o mais antigo cai quando passa disso.
const REMEMBERED: usize = 16;

/// Como um processo terminou.
#[derive(Clone, Copy, Debug)]
enum Ended {
    Exit(i64),
    Killed(KillCause),
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    /// Zero: posição vazia (`NEXT_PID` começa em 1).
    pid: u32,
    ended: Ended,
}

struct Ring {
    entries: [Entry; REMEMBERED],
    next: usize,
}

static ENDED: IrqLock<Ring> = IrqLock::new(Ring {
    entries: [Entry {
        pid: 0,
        ended: Ended::Exit(0),
    }; REMEMBERED],
    next: 0,
});
/// Quem espera um fim de processo: um bit por tarefa, sem alocar.
static WAITERS: Waiters = Waiters::new();

/// Chamado por `process::terminate`, na tarefa do processo, depois do log da morte.
pub fn finished(pid: u32, event: &ProcessEvent, no_memory: bool, kernel_half: bool) {
    let ended = if event.kind == event_kind::EXIT {
        Ended::Exit(event.code)
    } else {
        Ended::Killed(cause(event, no_memory, kernel_half))
    };
    {
        let mut ring = ENDED.lock();
        let at = ring.next;
        ring.entries[at] = Entry { pid, ended };
        ring.next = (at + 1) % REMEMBERED;
    }
    WAITERS.wake();
}

fn cause(event: &ProcessEvent, no_memory: bool, kernel_half: bool) -> KillCause {
    if no_memory {
        return KillCause::OutOfMemory;
    }
    match event.fault_kind {
        fault_kind::PAGE_FAULT => {
            let access = if event.access & fault_access::EXECUTE != 0 {
                Access::Execute
            } else if event.access & fault_access::WRITE != 0 {
                Access::Write
            } else {
                Access::Read
            };
            if kernel_half {
                KillCause::KernelPageFault { access }
            } else {
                KillCause::PageFault {
                    address: event.addr,
                    access,
                }
            }
        }
        fault_kind::ILLEGAL_INSTRUCTION => KillCause::IllegalInstruction,
        fault_kind::UNALIGNED => KillCause::Unaligned,
        fault_kind::ARITHMETIC => KillCause::Arithmetic,
        fault_kind::BREAKPOINT => KillCause::Breakpoint,
        _ => KillCause::Other,
    }
}

/// Tira da tabela o fim do processo `pid`, se já chegou.
fn take(pid: u32) -> Option<Ended> {
    let mut ring = ENDED.lock();
    let entry = ring.entries.iter_mut().find(|entry| entry.pid == pid)?;
    entry.pid = 0;
    Some(entry.ended)
}

/// Espera o fim do processo `pid`: a tarefa dorme até `finished` acordá-la, sem prazo.
fn wait_for(pid: u32) -> Ended {
    task::clear_permit();
    loop {
        let bit = WAITERS.register();
        let ended = take(pid);
        if ended.is_none() {
            task::block_until(u64::MAX);
        }
        WAITERS.unregister(bit);
        if let Some(ended) = ended {
            return ended;
        }
    }
}

/// Os arquivos do initramfs, em ordem, para o `ls`. O nome que não é texto não aparece.
pub fn list(archive: Option<&[u8]>, visit: &mut dyn FnMut(&ProgramInfo<'_>)) -> Listing {
    let Some(archive) = archive else {
        return Listing::NoInitramfs;
    };
    for entry in cpio::Reader::new(archive) {
        let Ok(entry) = entry else {
            return Listing::Unreadable;
        };
        if let Ok(name) = core::str::from_utf8(entry.name) {
            visit(&ProgramInfo {
                name,
                bytes: entry.data.len() as u64,
            });
        }
    }
    Listing::Done
}

/// Carrega `name` do initramfs e o roda como processo. Com `wait`, a tarefa do shell dorme até o
/// processo terminar.
pub fn run(
    frames: Frames,
    direct_map_offset: u64,
    archive: Option<&[u8]>,
    name: &str,
    wait: bool,
) -> RunOutcome {
    let Some(archive) = archive else {
        return RunOutcome::NoInitramfs;
    };
    let file = match loader::find(archive, name.as_bytes()) {
        Ok(file) => file,
        Err(LoadError::Missing) => return RunOutcome::NotFound,
        Err(_) => return RunOutcome::Failed,
    };
    let spawned = match process::spawn_as(frames, direct_map_offset, Source::Elf(file), 0, name) {
        Ok(spawned) => spawned,
        Err(error) => return refusal(&error),
    };
    if !wait {
        return RunOutcome::Started { pid: spawned.pid };
    }
    userout::direct();
    let ended = wait_for(spawned.pid);
    userout::queued();
    match ended {
        Ended::Exit(code) => RunOutcome::Exited {
            pid: spawned.pid,
            code,
        },
        Ended::Killed(cause) => RunOutcome::Killed {
            pid: spawned.pid,
            cause,
        },
    }
}

/// Por que o programa não virou processo, no vocabulário do shell.
fn refusal(error: &ProcessError) -> RunOutcome {
    use rustos_hal::MapError;

    use crate::task::TaskError;
    match error {
        ProcessError::Load(LoadError::Elf(_)) => RunOutcome::NotExecutable,
        ProcessError::Task(TaskError::NoSlot) => RunOutcome::TooManyProcesses,
        ProcessError::Load(LoadError::Map(MapError::OutOfFrames))
        | ProcessError::Task(TaskError::NoMemory | TaskError::Stack(_))
        | ProcessError::Handles(rustos_hal::abi::Status::NoMemory) => RunOutcome::NoMemory,
        _ => RunOutcome::Failed,
    }
}
