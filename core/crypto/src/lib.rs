//! Criptografia do núcleo: ChaCha20, BLAKE2b e o gerador de números aleatórios.
//! Puro: a semente do hardware e o estado global são do `kernel/`.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod blake2b;
pub mod chacha20;
pub mod rng;
pub mod seed;
mod wipe;

pub use rng::Rng;
pub use wipe::wipe;
