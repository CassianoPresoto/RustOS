//! Portas do hexágono: os contratos que o núcleo usa para falar com o hardware.
//! Só tipos e traits; quem implementa mora em `arch/`, `boot/` e `drivers/`.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod addr;
pub mod boot;
pub mod console;
pub mod cpu;
pub mod display;
pub mod input;
pub mod interrupt;
pub mod io;
pub mod memory;
pub mod paging;
pub mod time;

pub use addr::{PhysAddr, VirtAddr};
pub use boot::{BootInfo, BootTimeline, BootloaderId};
pub use console::Console;
pub use cpu::Cpu;
pub use display::{Channel, FramebufferInfo};
pub use input::{ByteSource, KeyCode, KeyEvent};
pub use interrupt::{Fault, FaultKind, Handlers, Irq};
pub use io::RegisterBus;
pub use memory::{MemoryRegion, RegionKind};
pub use paging::{CacheMode, FrameSource, MapError, PAGE_SIZE, PageFlags, PageMapper};
pub use time::Clock;
