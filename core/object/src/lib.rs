//! O modelo de objetos da fase 4, puro: tabela de handles, direitos, contagem de handles e a
//! fila do canal. O kernel liga os objetos reais a estes tipos (SDD 17, "Modelo de objetos").

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod channel;
pub mod object;
pub mod rights;
pub mod table;
pub mod transfer;

#[cfg(test)]
mod tests;

pub use channel::{Channel, Message, Peek, SendTicket};
pub use object::{
    EndId, HandleCount, Lock, Object, ObjectKind, Side, Transit, Transits, Worklist, close,
    close_pending, drain_dead,
};
pub use table::{
    FixedSlots, HandleTable, Limits, ReplaceFailed, Reservation, Slot, SlotStorage, TakeFailed,
    handle_from_raw,
};
pub use transfer::{ReadFailed, ReadSizes, channel_read, channel_write, discard_handles};
