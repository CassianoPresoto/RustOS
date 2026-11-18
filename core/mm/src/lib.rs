//! Gerência de memória: política pura, testável no host, sem `unsafe`.
//! Quem transforma endereço físico em memória acessível é o kernel.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

#[cfg(test)]
mod bench;
#[cfg(test)]
mod bench_bitmap;
mod bittree;
pub mod buddy;
pub mod cache;
pub mod frame;
pub mod heap;
pub mod layout;
pub mod memory;
pub mod ranges;
pub mod tables;
#[cfg(test)]
mod testing;
pub mod user;
pub mod watermark;
pub mod window;
pub mod wx;

pub use buddy::{BuddyAllocator, FrameAllocator, MAX_ORDER};
pub use cache::{Cached, FrameCache, FrameCaches};
pub use frame::{FRAME_SIZE, FrameError, PhysFrame};
pub use heap::{HeapError, KernelHeap};
pub use layout::{MemorySummary, find_free_block, summarize};
pub use memory::FrameMemory;
pub use tables::PageTables;
pub use user::{UserRange, UserRangeError};
pub use window::{AddressWindow, free_top_level_slot};
pub use wx::{Section, Violation, WxChecker};
