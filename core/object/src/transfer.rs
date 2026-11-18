//! `channel_write` e `channel_read` na ordem do SDD 17, sobre uma tabela e um canal trancados
//! pelo kernel (a ordem de lock é tabela, depois canal). Aqui nada tranca: `Lock` diz como.

use rustos_hal::abi::{CHANNEL_MAX_BYTES, CHANNEL_MAX_HANDLES, Handle, Rights, Status};

use crate::channel::{Message, Peek};
use crate::object::{
    Lock, Object, ObjectKind, Side, Transits, Worklist, close_pending, drain_dead,
};
use crate::rights::required;
use crate::table::{HandleTable, Reservation, SlotStorage};

/// O veredito da escrita e o que fechar depois de soltar o lock da tabela.
struct Outcome<T> {
    result: Result<(), Status>,
    discard: Transits<T>,
    /// A ponta que morreu na própria escrita: a fila dela se esvazia fora do lock.
    dead: Option<(T, Side)>,
}

impl<T> Outcome<T> {
    const fn ok() -> Self {
        Self {
            result: Ok(()),
            discard: Transits::new(),
            dead: None,
        }
    }

    const fn keep(status: Status) -> Self {
        Self {
            result: Err(status),
            discard: Transits::new(),
            dead: None,
        }
    }

    const fn consume(status: Status, discard: Transits<T>) -> Self {
        Self {
            result: Err(status),
            discard,
            dead: None,
        }
    }
}

/// O erro que consome o array sem olhar o canal (`InvalidArgs` por excesso de bytes ou de
/// handles, ou a falha ao copiar os bytes): os handles que existem saem da tabela e são fechados.
pub fn discard_handles<T, S, L, W>(table: &L, handles: &[Handle], work: &mut W)
where
    T: Object,
    S: SlotStorage<T>,
    L: Lock<HandleTable<T, S>>,
    W: Worklist<T>,
{
    for chunk in handles.chunks(CHANNEL_MAX_HANDLES) {
        let taken = table.with(|table| table.remove_existing(chunk));
        taken.close_all(work);
    }
    close_pending(work);
}

/// `channel_write(ch, bytes, handles)`. Os `handles` e os `bytes` já foram copiados do usuário
/// (a alocação é do kernel, antes). Atômica: a mensagem entra com todos os handles ou nenhum
/// fica a meio caminho. Todo erro **exceto** `ShouldWait` fecha os handles do array que existem
/// (inclusive o excesso de bytes ou de handles, `InvalidArgs`), fora do lock da tabela.
pub fn channel_write<T, S, L, W>(
    table: &L,
    channel: Handle,
    bytes: T::Bytes,
    handles: &[Handle],
    work: &mut W,
) -> Result<(), Status>
where
    T: Object + Clone,
    S: SlotStorage<T>,
    L: Lock<HandleTable<T, S>>,
    W: Worklist<T>,
{
    if handles.len() > CHANNEL_MAX_HANDLES || bytes.as_ref().len() > CHANNEL_MAX_BYTES {
        discard_handles(table, handles, work);
        return Err(Status::InvalidArgs);
    }
    let outcome = table.with(|table| write_locked(table, channel, bytes, handles));
    outcome.discard.close_all(work);
    if let Some((object, side)) = &outcome.dead {
        drain_dead(object, *side, work);
    }
    close_pending(work);
    outcome.result
}

fn cancel_ticket<T: Object, S: SlotStorage<T>>(
    table: &HandleTable<T, S>,
    channel: Handle,
    ticket: crate::channel::SendTicket,
) {
    let granted = table
        .get(channel, Rights::NONE)
        .expect("o canal segue na tabela até o commit");
    granted.object.with_channel(|chan| chan.cancel(ticket));
}

fn write_locked<T, S>(
    table: &mut HandleTable<T, S>,
    channel: Handle,
    bytes: T::Bytes,
    handles: &[Handle],
) -> Outcome<T>
where
    T: Object + Clone,
    S: SlotStorage<T>,
{
    let (end, reserved) = match table.get_of(channel, ObjectKind::Channel, required::CHANNEL_WRITE)
    {
        Ok(granted) => match granted.object.channel_end() {
            Some(end) => (
                end,
                granted.object.with_channel(|chan| chan.reserve(end.side)),
            ),
            None => return Outcome::consume(Status::WrongType, table.remove_existing(handles)),
        },
        Err(status) => return Outcome::consume(status, table.remove_existing(handles)),
    };
    let ticket = match reserved {
        Some(Ok(ticket)) => ticket,
        Some(Err(Status::ShouldWait)) => return Outcome::keep(Status::ShouldWait),
        Some(Err(status)) => return Outcome::consume(status, table.remove_existing(handles)),
        None => return Outcome::consume(Status::WrongType, table.remove_existing(handles)),
    };
    let mut items = [(Handle::INVALID, Rights::NONE); CHANNEL_MAX_HANDLES];
    for (item, &handle) in items.iter_mut().zip(handles) {
        *item = (handle, required::TRANSFER);
    }
    if let Err(status) = table.check_array(&items[..handles.len()], Some(end)) {
        cancel_ticket(table, channel, ticket);
        return Outcome::consume(status, table.remove_existing(handles));
    }
    let taken = table.remove_existing(handles);
    let message = match Message::new(bytes, taken) {
        Ok(message) => message,
        Err((status, taken)) => {
            cancel_ticket(table, channel, ticket);
            return Outcome::consume(status, taken);
        }
    };
    let granted = table
        .get(channel, Rights::NONE)
        .expect("o canal segue na tabela até o commit");
    match granted
        .object
        .with_channel(|chan| chan.commit(ticket, message))
    {
        Some(Ok(false)) => Outcome::ok(),
        Some(Ok(true)) => {
            granted.object.on_end_died(end.side.other());
            Outcome {
                dead: Some((granted.object.clone(), end.side.other())),
                ..Outcome::ok()
            }
        }
        Some(Err(message)) => {
            let (_, discard) = message.into_parts();
            Outcome::consume(Status::PeerClosed, discard)
        }
        None => unreachable!("o canal é uma ponta"),
    }
}

/// Os tamanhos de uma mensagem: bytes e handles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadSizes {
    pub bytes: usize,
    pub handles: usize,
}

/// `channel_read` falhou. Em `BufferTooSmall`, `actual` traz o que a mensagem pede.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadFailed {
    pub status: Status,
    pub actual: ReadSizes,
}

impl ReadFailed {
    const fn status(status: Status) -> Self {
        Self {
            status,
            actual: ReadSizes {
                bytes: 0,
                handles: 0,
            },
        }
    }
}

type Reserved = [Option<Reservation>; CHANNEL_MAX_HANDLES];

fn cancel_all<T: Object, S: SlotStorage<T>>(
    table: &mut HandleTable<T, S>,
    reserved: &mut Reserved,
) {
    for slot in reserved.iter_mut() {
        if let Some(reservation) = slot.take() {
            table.cancel(reservation);
        }
    }
}

fn peek_locked<T: Object, S: SlotStorage<T>>(
    table: &HandleTable<T, S>,
    channel: Handle,
) -> Result<Peek<T>, Status> {
    let granted = table.get_of(channel, ObjectKind::Channel, required::CHANNEL_READ)?;
    let end = granted.object.channel_end().ok_or(Status::WrongType)?;
    granted
        .object
        .with_channel(|chan| chan.peek(end.side))
        .ok_or(Status::WrongType)?
}

fn reserve_all<T: Object, S: SlotStorage<T>>(
    table: &mut HandleTable<T, S>,
    count: usize,
    reserved: &mut Reserved,
    values: &mut [Handle; CHANNEL_MAX_HANDLES],
) -> Result<(), Status> {
    for index in 0..count {
        match table.reserve() {
            Ok(reservation) => {
                values[index] = reservation.handle();
                reserved[index] = Some(reservation);
            }
            Err(status) => {
                cancel_all(table, reserved);
                return Err(status);
            }
        }
    }
    Ok(())
}

enum Commit {
    Done,
    Retry,
    Gone,
}

fn commit_read<T: Object, S: SlotStorage<T>>(
    table: &mut HandleTable<T, S>,
    channel: Handle,
    seq: u32,
    reserved: &mut Reserved,
) -> Commit {
    let popped = match table.get_of(channel, ObjectKind::Channel, required::CHANNEL_READ) {
        Ok(granted) => granted.object.channel_end().and_then(|end| {
            granted
                .object
                .with_channel(|chan| chan.pop_if(end.side, seq))
        }),
        Err(_) => None,
    };
    let message = match popped {
        None => {
            cancel_all(table, reserved);
            return Commit::Gone;
        }
        Some(None) => {
            cancel_all(table, reserved);
            return Commit::Retry;
        }
        Some(Some(message)) => message,
    };
    let (_, mut transits) = message.into_parts();
    for (index, slot) in reserved.iter_mut().enumerate().take(transits.len()) {
        let transit = transits.take(index).expect("a mensagem tem o handle");
        let reservation = slot.take().expect("um slot reservado por handle");
        let _ = table.commit_moved(reservation, transit);
    }
    Commit::Done
}

/// `channel_read(ch, ...)`, em cinco passos (SDD 17): espia a frente; reserva os slots dos
/// handles; copia para o usuário **sem lock** por `copy_out(bytes, handles)`, que devolve `false`
/// se a cópia falhou; retira a mensagem só se a frente não mudou, senão libera as reservas e
/// repete desde o início (lacuna L2). Nenhuma falha perde mensagem nem handle.
pub fn channel_read<T, S, L, F>(
    table: &L,
    channel: Handle,
    bytes_cap: usize,
    handles_cap: usize,
    mut copy_out: F,
) -> Result<ReadSizes, ReadFailed>
where
    T: Object,
    S: SlotStorage<T>,
    L: Lock<HandleTable<T, S>>,
    F: FnMut(&[u8], &[Handle]) -> bool,
{
    loop {
        let peek = table
            .with(|table| peek_locked(table, channel))
            .map_err(ReadFailed::status)?;
        let sizes = ReadSizes {
            bytes: peek.bytes.as_ref().len(),
            handles: peek.handles,
        };
        if sizes.bytes > bytes_cap || sizes.handles > handles_cap {
            return Err(ReadFailed {
                status: Status::BufferTooSmall,
                actual: sizes,
            });
        }
        let mut reserved: Reserved = [const { None }; CHANNEL_MAX_HANDLES];
        let mut values = [Handle::INVALID; CHANNEL_MAX_HANDLES];
        table
            .with(|table| reserve_all(table, sizes.handles, &mut reserved, &mut values))
            .map_err(ReadFailed::status)?;
        if !copy_out(peek.bytes.as_ref(), &values[..sizes.handles]) {
            table.with(|table| cancel_all(table, &mut reserved));
            return Err(ReadFailed::status(Status::OutOfRange));
        }
        match table.with(|table| commit_read(table, channel, peek.seq, &mut reserved)) {
            Commit::Done => return Ok(sizes),
            Commit::Retry => {}
            Commit::Gone => return Err(ReadFailed::status(Status::BadHandle)),
        }
    }
}
