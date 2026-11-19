//! As chamadas de handle, canal e espera da ABI nativa (SDD 17, fatia 4c). Cada uma valida o handle
//! e os direitos na tabela do processo, copia de e para o usuário sem lock preso e, nas que
//! devolvem handle por ponteiro, reserva, copia e só então confirma (lacuna L4).

use rustos_hal::abi::{
    CHANNEL_MAX_BYTES, Handle, ReadActual, Rights, Status, SyscallReturn, USER_HANDLE_ARRAY_MAX,
    signals, wait_deadline,
};
use rustos_mm::UserRange;
use rustos_object::rights::{self, required};
use rustos_object::{
    ReadFailed, ReplaceFailed, channel_read, channel_write, close, discard_handles, handle_from_raw,
};

use crate::object::{Bytes, Charge, Obj, Work, new_channel};
use crate::process::{self, Process};
use crate::{platform, task};

pub(super) type Outcome = Result<u64, Status>;

/// A resposta da chamada: o valor em `rdx`/`x1` ou o erro.
pub fn reply(outcome: Outcome) -> SyscallReturn {
    match outcome {
        Ok(value) => SyscallReturn::ok(value),
        Err(status) => SyscallReturn::err(status),
    }
}

/// Roda `f` com o processo que chamou. Syscall sem processo não existe; `BadState` é a rede.
pub(super) fn current<R>(f: impl FnOnce(&Process) -> Result<R, Status>) -> Result<R, Status> {
    process::with_current(f).unwrap_or(Err(Status::BadState))
}

/// O valor cru como handle; o que não tem forma de handle vira `INVALID`, que a tabela recusa
/// com `BadHandle` (e, num array que se move, consome os outros).
pub(super) fn lenient(raw: u64) -> Handle {
    handle_from_raw(raw).unwrap_or(Handle::INVALID)
}

pub(super) fn u32_arg(raw: u64) -> Result<u32, Status> {
    u32::try_from(raw).map_err(|_| Status::InvalidArgs)
}

/// `handle_close(h)`: remove sob o lock, fecha depois dele.
pub fn handle_close(raw: u64) -> Outcome {
    let handle = handle_from_raw(raw)?;
    let transit = current(|p| p.table().lock().remove(handle))?;
    close(transit, &mut Work::new());
    Ok(0)
}

/// `handle_duplicate(h, rights)`: `DUPLICATE` em `h`, direitos novos dentro dos dele.
pub fn handle_duplicate(raw: u64, bits: u64) -> Outcome {
    let handle = handle_from_raw(raw)?;
    let wanted = rights::parse(u32_arg(bits)?)?;
    current(|p| p.table().lock().duplicate(handle, wanted)).map(Handle::raw)
}

/// `handle_replace(h, rights)`: o velho sai sempre que existe, e o fechamento é fora do lock.
pub fn handle_replace(raw: u64, bits: u64) -> Outcome {
    let handle = handle_from_raw(raw)?;
    let wanted = u32_arg(bits).and_then(rights::parse);
    let replaced = current(|p| {
        let mut table = p.table().lock();
        Ok(match wanted {
            Ok(wanted) => table.replace(handle, wanted),
            Err(status) => Err(match table.remove(handle) {
                Ok(closed) => ReplaceFailed {
                    status,
                    closed: Some(closed),
                },
                Err(status) => ReplaceFailed {
                    status,
                    closed: None,
                },
            }),
        })
    })?;
    match replaced {
        Ok(new) => Ok(new.raw()),
        Err(ReplaceFailed { status, closed }) => {
            if let Some(transit) = closed {
                close(transit, &mut Work::new());
            }
            Err(status)
        }
    }
}

/// `debug_write` pede um handle de console com `WRITE`.
pub fn check_console(raw: u64) -> Result<(), Status> {
    let handle = handle_from_raw(raw)?;
    current(|p| {
        let table = p.table().lock();
        table
            .get_of(
                handle,
                rustos_object::ObjectKind::Console,
                required::DEBUG_WRITE,
            )
            .map(|_| ())
    })
}

/// `object_wait_one(h, signals, deadline_ns)`: devolve os sinais que valem. `signals` sem bit ou com
/// bit que a ABI não define é `InvalidArgs`; o prazo é absoluto no relógio monotônico.
pub fn object_wait_one(raw: u64, wanted: u64, deadline_ns: u64) -> Outcome {
    let handle = handle_from_raw(raw)?;
    let wanted = u32_arg(wanted)?;
    let known = signals::READABLE | signals::WRITABLE | signals::PEER_CLOSED | signals::TERMINATED;
    if wanted == 0 || wanted & !known != 0 {
        return Err(Status::InvalidArgs);
    }
    let object = current(|p| {
        let table = p.table().lock();
        table
            .get(handle, required::OBJECT_WAIT)
            .map(|granted| granted.object.clone())
    })?;
    wait(&object, wanted, deadline_ns).map(u64::from)
}

fn wait(object: &Obj, wanted: u32, deadline_ns: u64) -> Result<u32, Status> {
    task::clear_permit();
    loop {
        if process::with_current(Process::is_dying) == Some(true) {
            return Err(Status::BadState);
        }
        let registered = object
            .waiters()
            .map(|waiters| (waiters, waiters.register()));
        let observed = object.signals();
        let now = platform::now_ns();
        let until = now
            .and_then(|now| wait_deadline(now, deadline_ns))
            .unwrap_or(u64::MAX);
        if observed & wanted == 0 && now.is_some_and(|now| now < until) {
            task::block_until(until);
        }
        if let Some((waiters, bit)) = registered {
            waiters.unregister(bit);
        }
        if observed & wanted != 0 {
            return Ok(observed);
        }
        match now {
            None => return Err(Status::NotSupported),
            Some(now) if until <= now => return Err(Status::TimedOut),
            Some(_) => {}
        }
    }
}

/// `channel_create(flags, out)`: dois handles em `out`. Reserva os slots, cria, copia e só então
/// confirma: falha na cópia deixa a tabela e a cobrança como estavam.
pub fn channel_create(flags: u64, out: u64) -> Outcome {
    if flags != 0 {
        return Err(Status::InvalidArgs);
    }
    let range = UserRange::new(out, 16).map_err(|_| Status::OutOfRange)?;
    current(|p| {
        let (first, second) = {
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
        let cancel = |first, second| {
            let mut table = p.table().lock();
            table.cancel(first);
            table.cancel(second);
        };
        let charges = (Charge::new(p.quota()), Charge::new(p.quota()));
        let (Some(charge_a), Some(charge_b)) = charges else {
            cancel(first, second);
            return Err(Status::NoMemory);
        };
        let (end_a, end_b) = new_channel([Some(charge_a), Some(charge_b)]);
        let mut values = [0_u8; 16];
        values[..8].copy_from_slice(&first.handle().raw().to_le_bytes());
        values[8..].copy_from_slice(&second.handle().raw().to_le_bytes());
        if !platform::copy_to_user(range, &values) {
            cancel(first, second);
            return Err(Status::OutOfRange);
        }
        let mut table = p.table().lock();
        table.commit(first, end_a, Rights::CHANNEL);
        table.commit(second, end_b, Rights::CHANNEL);
        Ok(0)
    })
}

/// O array de handles do usuário, copiado uma vez para o kernel.
fn copy_handles(ptr: u64, count: u64) -> Result<([Handle; USER_HANDLE_ARRAY_MAX], usize), Status> {
    let mut out = [Handle::INVALID; USER_HANDLE_ARRAY_MAX];
    let count = usize::try_from(count).map_err(|_| Status::InvalidArgs)?;
    if count > USER_HANDLE_ARRAY_MAX {
        return Err(Status::InvalidArgs);
    }
    if count == 0 {
        return Ok((out, 0));
    }
    let range = UserRange::new(ptr, count as u64 * 8).map_err(|_| Status::OutOfRange)?;
    let mut raw = [0_u8; USER_HANDLE_ARRAY_MAX * 8];
    if !platform::copy_from_user(range, &mut raw[..count * 8]) {
        return Err(Status::OutOfRange);
    }
    for (slot, chunk) in out.iter_mut().zip(raw.as_chunks::<8>().0).take(count) {
        *slot = lenient(u64::from_le_bytes(*chunk));
    }
    Ok((out, count))
}

/// `channel_write(ch, bytes, nbytes, handles, nhandles)`: consome o array em todo erro, menos
/// `ShouldWait` e a falha ao ler o próprio array.
pub fn channel_write_call(args: &[u64; 6]) -> Outcome {
    let [channel, bytes_ptr, nbytes, handles_ptr, nhandles, _] = *args;
    let (handles, count) = copy_handles(handles_ptr, nhandles)?;
    let handles = &handles[..count];
    let discard = |status: Status| {
        current(|p| {
            discard_handles(p.table(), handles, &mut Work::new());
            Ok(())
        })
        .ok();
        Err(status)
    };
    let Ok(nbytes) = usize::try_from(nbytes) else {
        return discard(Status::InvalidArgs);
    };
    if nbytes > CHANNEL_MAX_BYTES {
        return discard(Status::InvalidArgs);
    }
    let mut buffer = [0_u8; CHANNEL_MAX_BYTES];
    if nbytes > 0 {
        let Ok(range) = UserRange::new(bytes_ptr, nbytes as u64) else {
            return discard(Status::OutOfRange);
        };
        if !platform::copy_from_user(range, &mut buffer[..nbytes]) {
            return discard(Status::OutOfRange);
        }
    }
    let mut charge_failed = false;
    let written = current(|p| {
        let Some(bytes) = Bytes::charged(p.quota(), &buffer[..nbytes]) else {
            charge_failed = true;
            return Ok(Ok(()));
        };
        Ok(channel_write(
            p.table(),
            lenient(channel),
            bytes,
            handles,
            &mut Work::new(),
        ))
    })?;
    if charge_failed {
        return discard(Status::NoMemory);
    }
    written.map(|()| 0)
}

/// `channel_read(ch, bytes, bytes_cap, handles, handles_cap, actual*)`. Os tamanhos e os valores
/// dos handles vão ao usuário antes de a mensagem sair da fila.
pub fn channel_read_call(args: &[u64; 6]) -> Outcome {
    let [
        channel,
        bytes_ptr,
        bytes_cap,
        handles_ptr,
        handles_cap,
        actual_ptr,
    ] = *args;
    let channel = handle_from_raw(channel)?;
    let actual_range = UserRange::new(actual_ptr, 16).map_err(|_| Status::OutOfRange)?;
    let cap = |raw: u64| usize::try_from(raw).unwrap_or(usize::MAX);
    let put_actual = |actual: ReadActual| {
        let mut raw = [0_u8; 16];
        raw[..8].copy_from_slice(&actual.bytes.to_le_bytes());
        raw[8..].copy_from_slice(&actual.handles.to_le_bytes());
        platform::copy_to_user(actual_range, &raw)
    };
    let copy_out = |bytes: &[u8], handles: &[Handle]| {
        let actual = ReadActual {
            bytes: bytes.len() as u64,
            handles: handles.len() as u64,
        };
        let bytes_ok = bytes.is_empty()
            || UserRange::new(bytes_ptr, bytes.len() as u64)
                .is_ok_and(|range| platform::copy_to_user(range, bytes));
        bytes_ok && copy_handles_out(handles_ptr, handles) && put_actual(actual)
    };
    let read = current(|p| {
        Ok(channel_read(
            p.table(),
            channel,
            cap(bytes_cap),
            cap(handles_cap),
            copy_out,
        ))
    })?;
    match read {
        Ok(_) => Ok(0),
        Err(ReadFailed {
            status: Status::BufferTooSmall,
            actual,
        }) => {
            let told = put_actual(ReadActual {
                bytes: actual.bytes as u64,
                handles: actual.handles as u64,
            });
            Err(if told {
                Status::BufferTooSmall
            } else {
                Status::OutOfRange
            })
        }
        Err(failed) => Err(failed.status),
    }
}

fn copy_handles_out(ptr: u64, handles: &[Handle]) -> bool {
    if handles.is_empty() {
        return true;
    }
    let Ok(range) = UserRange::new(ptr, handles.len() as u64 * 8) else {
        return false;
    };
    let mut raw = [0_u8; USER_HANDLE_ARRAY_MAX * 8];
    for (chunk, handle) in raw.as_chunks_mut::<8>().0.iter_mut().zip(handles) {
        chunk.copy_from_slice(&handle.raw().to_le_bytes());
    }
    platform::copy_to_user(range, &raw[..handles.len() * 8])
}
