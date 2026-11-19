//! As chamadas de processo e thread da ABI nativa (SDD 17, fatia 4c, parte 3): `process_create`,
//! `thread_create` e `process_kill` (`thread_exit` e `process_exit` não pedem handle). Quem cria
//! paga a cobrança do objeto à própria conta de bytes, e erro depois de o kernel conferir o pai
//! consome os handles que se movem.

use alloc::sync::Arc;

use rustos_hal::abi::{Handle, PROCESS_NAME_MAX, Rights, Status, USER_MIN, USER_TOP};
use rustos_mm::UserRange;
use rustos_object::rights::required;
use rustos_object::{ObjectKind, TakeFailed, Transits, handle_from_raw};

use super::handles::{Outcome, current, lenient};
use crate::object::{ByteCharge, Obj, Work};
use crate::platform;
use crate::process::{self, Entry, PROCESS_BYTES, Process, THREAD_BYTES, ThreadObj, VmarObj};

/// O processo de um handle com `rights`, para quem age sobre outro processo.
fn process_of(p: &Process, raw: u64, rights: Rights) -> Result<Arc<Process>, Status> {
    let table = p.table().lock();
    let granted = table.get_of(handle_from_raw(raw)?, ObjectKind::Process, rights)?;
    match granted.object {
        Obj::Process(target) => Ok(Arc::clone(target)),
        _ => Err(Status::WrongType),
    }
}

/// Fecha os handles que um erro deixou fora da tabela, sem lock nenhum preso.
fn close_taken(taken: Transits<Obj>) {
    taken.close_all(&mut Work::new());
}

/// `process_create(parent, name_ptr, name_len, bootstrap, events, out)`: um processo vazio, sem
/// thread: o espaço, o VMAR raiz e o `bootstrap` na tabela dele, como o primeiro handle (valor 1).
/// `parent` exige `MANAGE_PROCESS`; `bootstrap`, `TRANSFER`. O canal de eventos é da 4d: `events`
/// diferente de 0 é `NotSupported`. Processo e VMAR raiz voltam em `out`, dois `u64`.
pub fn process_create(args: &[u64; 6]) -> Outcome {
    let [parent, name_ptr, name_len, bootstrap, events, out] = *args;
    let items = [
        (lenient(bootstrap), required::PROCESS_CREATE_BOOTSTRAP),
        (lenient(events), required::PROCESS_CREATE_EVENTS),
    ];
    let moved = if events == 0 { &items[..1] } else { &items[..] };
    let taken = current(|p| {
        let mut table = p.table().lock();
        let authorized = table
            .get_of(
                lenient(parent),
                ObjectKind::Process,
                required::PROCESS_CREATE_PARENT,
            )
            .map(|_| ());
        let taken = table.take_array(moved, None);
        Ok(match (authorized, taken) {
            (Ok(()), Ok(taken)) => Ok(taken),
            (Err(status), Ok(taken)) => Err(TakeFailed { status, taken }),
            (Err(status), Err(failed)) => Err(TakeFailed {
                status,
                taken: failed.taken,
            }),
            (Ok(()), Err(failed)) => Err(failed),
        })
    })?;
    let mut taken = match taken {
        Ok(taken) => taken,
        Err(failed) => {
            close_taken(failed.taken);
            return Err(failed.status);
        }
    };
    let result = create_child(name_ptr, name_len, out, events != 0, &mut taken);
    close_taken(taken);
    result
}

/// O resto de `process_create`, depois de o pai conferido e os handles tirados da tabela. O
/// `bootstrap` sai de `taken` só quando vai ao filho; o que sobra o chamador fecha.
fn create_child(
    name_ptr: u64,
    name_len: u64,
    out: u64,
    has_events: bool,
    taken: &mut Transits<Obj>,
) -> Outcome {
    if has_events {
        return Err(Status::NotSupported);
    }
    let name_len = usize::try_from(name_len).map_err(|_| Status::InvalidArgs)?;
    if name_len > PROCESS_NAME_MAX {
        return Err(Status::InvalidArgs);
    }
    let out_range = UserRange::new(out, 16).map_err(|_| Status::OutOfRange)?;
    if name_len > 0 {
        UserRange::new(name_ptr, name_len as u64).map_err(|_| Status::OutOfRange)?;
    }
    current(|p| {
        let slots = {
            let mut table = p.table().lock();
            let first = table.reserve()?;
            match table.reserve() {
                Ok(second) => (first, second),
                Err(status) => {
                    table.cancel(first);
                    return Err(status);
                }
            }
        };
        let cancel = |slots: (_, _)| {
            let mut table = p.table().lock();
            table.cancel(slots.0);
            table.cancel(slots.1);
        };
        let Some(cost) = ByteCharge::new(p.quota(), PROCESS_BYTES) else {
            cancel(slots);
            return Err(Status::NoMemory);
        };
        let child = match Process::create_empty(cost) {
            Ok(child) => child,
            Err(status) => {
                cancel(slots);
                return Err(status);
            }
        };
        if let Some(bootstrap) = taken.take(0) {
            let mut table = child.table().lock();
            match table.reserve() {
                Ok(slot) => {
                    table.commit_moved(slot, bootstrap);
                }
                Err(status) => {
                    drop(table);
                    rustos_object::close(bootstrap, &mut Work::new());
                    cancel(slots);
                    return Err(status);
                }
            }
        }
        let vmar = VmarObj::new(&child);
        let mut values = [0_u8; 16];
        values[..8].copy_from_slice(&slots.0.handle().raw().to_le_bytes());
        values[8..].copy_from_slice(&slots.1.handle().raw().to_le_bytes());
        if !platform::copy_to_user(out_range, &values) {
            cancel(slots);
            return Err(Status::OutOfRange);
        }
        let mut table = p.table().lock();
        table.commit(slots.0, Obj::Process(child), Rights::PROCESS);
        table.commit(slots.1, Obj::Vmar(vmar), Rights::VMAR);
        Ok(0)
    })
}

/// `thread_create(process, entry, sp, arg0, arg1)`: cria **e inicia** uma thread em `process`
/// (`MANAGE_THREAD`); a primeira inicia o processo. `entry` em `[USER_MIN, USER_TOP)`, `sp` até
/// `USER_TOP` e alinhado a 16, senão `InvalidArgs`. `arg0` e `arg1` vão a `rdi`/`rsi` (`x0`/`x1`).
pub fn thread_create(call: &[u64; 6]) -> Outcome {
    let [process, entry, sp, first, second, _] = *call;
    let target = current(|p| process_of(p, process, required::THREAD_CREATE))?;
    if !(USER_MIN..USER_TOP).contains(&entry) || sp > USER_TOP || !sp.is_multiple_of(16) {
        return Err(Status::InvalidArgs);
    }
    current(|p| {
        let slot = p.table().lock().reserve()?;
        let Some(cost) = ByteCharge::new(p.quota(), THREAD_BYTES) else {
            p.table().lock().cancel(slot);
            return Err(Status::NoMemory);
        };
        let thread = ThreadObj::new(&target, Some(cost));
        let at = Entry {
            pc: entry,
            sp,
            arg0: first,
            arg1: second,
        };
        if let Err(status) = process::start_thread(&target, Arc::clone(&thread), at, 0) {
            p.table().lock().cancel(slot);
            return Err(status);
        }
        let handle: Handle = p
            .table()
            .lock()
            .commit(slot, Obj::Thread(thread), Rights::THREAD);
        Ok(handle.raw())
    })
}

/// `process_kill(process)`: o processo morre com o código de kill; as threads são paradas (as de
/// outras CPUs por IPI) e a última a sair desfaz o espaço. `DESTROY`; `BadState` se já morria.
pub fn process_kill(process: u64) -> Outcome {
    let target = current(|p| process_of(p, process, required::PROCESS_KILL))?;
    target.kill().map(|()| 0)
}
