//! O que o shell faz com programas do initramfs (SDD 17, "O M1"): listar os arquivos e rodar um
//! como processo, esperando ou não. O fim de cada processo chega por `finished`, numa tabela curta
//! que quem espera consulta; o processo morre na própria tarefa, sem ninguém o esperar por handle.

use alloc::sync::Arc;

use rustos_cpio as cpio;
use rustos_hal::abi::{
    ARGS_BYTES_MAX, ARGS_MAX, ProcessEvent, event_kind, fault_access, fault_kind,
};
use rustos_shell::{Access, KillCause, KillOutcome, Listing, ProgramInfo, RunOutcome};

use crate::frames::Frames;
use crate::interrupts;
use crate::loader::{self, LoadError};
use crate::object::Waiters;
use crate::process::{self, Process, ProcessError, Source};
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

/// Espera o fim do processo `pid`: a tarefa dorme até `finished` ou um Ctrl-C acordá-la, sem prazo.
/// Ctrl-C mata o processo e a espera segue até o fim dele; o bool diz que houve interrupção.
fn wait_for(pid: u32, process: &Arc<Process>) -> (Ended, bool) {
    interrupts::clear_interrupt();
    task::clear_permit();
    let mut interrupted = false;
    loop {
        let bit = WAITERS.register();
        // Armar a entrada antes de olhar a flag: um Ctrl-C que chegue agora acorda a tarefa.
        task::watch_input();
        let ended = take(pid);
        // Toda volta consome a flag e descarta a entrada: um segundo Ctrl-C não sobra como `^C` no
        // prompt. `BadState` (já morria por conta própria) mostra a causa real, não `interrupted`.
        if interrupts::take_interrupt() {
            if ended.is_none() {
                interrupted |= process.kill().is_ok();
            }
            interrupts::flush_input();
            if ended.is_none() {
                WAITERS.unregister(bit);
                continue;
            }
        }
        if ended.is_none() {
            task::block_until(u64::MAX);
        }
        WAITERS.unregister(bit);
        if let Some(ended) = ended {
            return (ended, interrupted);
        }
    }
}

/// Mata o processo `pid` que ainda vive: `Killed`, `NotFound` se não há, `AlreadyEnding` se já morre.
pub fn kill(pid: u32) -> KillOutcome {
    let Some(process) = process::find(pid) else {
        return KillOutcome::NotFound;
    };
    match process.kill() {
        Ok(()) => KillOutcome::Killed,
        Err(_) => KillOutcome::AlreadyEnding,
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

/// Carrega `name` do initramfs e o roda como processo, com `args` depois do nome (que é `argv[0]`).
/// Com `wait`, a tarefa do shell dorme até o processo terminar.
pub fn run(
    frames: Frames,
    direct_map_offset: u64,
    archive: Option<&[u8]>,
    name: &str,
    extra: &[&str],
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
    let Some((argv, count)) = argv_of(name, extra) else {
        return too_many_arguments();
    };
    // O programa do shell recebe o framebuffer, se a tela é XRGB (SDD 18); o `init` não.
    let options = process::Options {
        framebuffer: true,
        ..process::Options::default()
    };
    let spawned = match process::spawn_with(
        frames,
        direct_map_offset,
        Source::Elf(file),
        0,
        options,
        name,
        &argv[..count],
    ) {
        Ok((spawned, _)) => spawned,
        Err(error) => return refusal(&error),
    };
    if !wait {
        return RunOutcome::Started { pid: spawned.pid };
    }
    userout::direct();
    let (ended, interrupted) = wait_for(spawned.pid, &spawned.process);
    userout::queued();
    if interrupted {
        return RunOutcome::Interrupted { pid: spawned.pid };
    }
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

/// O `argv` de `run`: o nome do programa e cada argumento, na ordem, e quantas palavras são. `None` se
/// não cabe em `ARGS_MAX`; o tamanho das cadeias é conferido pelo `spawn`.
fn argv_of<'a>(name: &'a str, extra: &[&'a str]) -> Option<([&'a [u8]; ARGS_MAX], usize)> {
    let count = extra
        .len()
        .checked_add(1)
        .filter(|&count| count <= ARGS_MAX)?;
    let mut argv: [&[u8]; ARGS_MAX] = [&[]; ARGS_MAX];
    argv[0] = name.as_bytes();
    for (slot, arg) in argv[1..count].iter_mut().zip(extra) {
        *slot = arg.as_bytes();
    }
    Some((argv, count))
}

/// O `argv` passa dos tetos: o shell diz quais são.
fn too_many_arguments() -> RunOutcome {
    RunOutcome::TooManyArguments {
        max_words: ARGS_MAX,
        max_bytes: ARGS_BYTES_MAX,
    }
}

/// Por que o programa não virou processo, no vocabulário do shell.
fn refusal(error: &ProcessError) -> RunOutcome {
    use rustos_hal::MapError;

    use crate::task::TaskError;
    match error {
        ProcessError::Arguments => too_many_arguments(),
        ProcessError::Load(LoadError::Elf(_)) => RunOutcome::NotExecutable,
        ProcessError::Task(TaskError::NoSlot) => RunOutcome::TooManyProcesses,
        ProcessError::Load(LoadError::Map(MapError::OutOfFrames))
        | ProcessError::Task(TaskError::NoMemory | TaskError::Stack(_))
        | ProcessError::Handles(rustos_hal::abi::Status::NoMemory) => RunOutcome::NoMemory,
        _ => RunOutcome::Failed,
    }
}
