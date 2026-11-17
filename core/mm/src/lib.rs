//! Gerência de memória: política pura, testável no host, sem `unsafe`.
//! Quem transforma endereço físico em memória acessível é o kernel.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod frame;
pub mod layout;
pub mod window;

pub use frame::{BitmapFrameAllocator, FRAME_SIZE, FrameError, PhysFrame};
pub use layout::{MemorySummary, find_free_block, summarize};
pub use window::AddressWindow;
