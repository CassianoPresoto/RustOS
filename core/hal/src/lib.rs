//! Portas do hexágono: os contratos que o núcleo usa para falar com o hardware.
//! Tipos, traits e lógica pura sem estado de hardware (`clock`, `cmdline`, `redact`);
//! quem implementa as portas mora em `arch/`, `boot/` e `drivers/`.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod abi;
pub mod addr;
pub mod boot;
pub mod clock;
pub mod cmdline;
pub mod console;
pub mod cpu;
pub mod display;
pub mod entropy;
pub mod input;
pub mod interrupt;
pub mod io;
pub mod memory;
pub mod paging;
pub mod redact;

pub use addr::{PhysAddr, VirtAddr};
pub use boot::{BootInfo, BootTimeline, BootloaderId, CpuDescriptor, KernelImage};
pub use clock::TickScale;
pub use cmdline::{BootParams, EarlyPanic};
pub use console::Console;
pub use cpu::{Cpu, CpuLocal, Ipi, IpiKind, MAX_CPUS};
pub use display::{Channel, FramebufferInfo};
pub use entropy::HardwareEntropy;
pub use input::{ByteSource, KeyCode, KeyEvent};
pub use interrupt::{Fault, FaultKind, Handlers, Irq};
pub use io::RegisterBus;
pub use memory::{MemoryRegion, RegionKind, STACK_PAINT};
pub use paging::{
    AddressSpace, CacheMode, FrameSource, LARGE_PAGE_SIZE, MapError, Mapping, MmuControl,
    PAGE_SIZE, PageFlags, PageFormat, PageMapper, TableMemory,
};
