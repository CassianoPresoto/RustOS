//! Primitivas de sincronização sem conhecimento de domínio.
//! Moram fora de `core/` porque precisam de `unsafe` para existir.

#![cfg_attr(not(test), no_std)]

mod ring;
mod spin;

pub use ring::ByteRing;
pub use spin::{SpinLock, SpinLockGuard};
