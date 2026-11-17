//! Adaptador da ISA aarch64: o único lugar com assembly de ARM.
//! O que tem `asm!` só compila para o alvo bare-metal; o resto é testável no host.

#![cfg_attr(not(test), no_std)]

pub mod descriptor;
pub mod esr;
pub mod features;
pub mod intid;
pub mod sgi;
pub mod time;

#[cfg(all(target_arch = "aarch64", target_os = "none"))]
mod context;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
mod cpu;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
mod entropy;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub mod exception;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub mod fpu;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub mod gic_cpu;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
mod mmio;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
mod mmu;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
mod percpu;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub mod protection;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
mod stack;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub mod timer;

#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub use context::{Context, switch};
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub use cpu::Aarch64Cpu;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub use entropy::Aarch64Entropy;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub use gic_cpu::Aarch64Ipi;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub use mmio::MmioBus;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub use mmu::{Aarch64Mmu, DirectMapTables, active_root, prepare_space};
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub use percpu::Aarch64CpuLocal;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub use stack::{Continuation, call_on_stack};

/// PL011, distribuidor e redistribuidor do GICv3 da máquina `virt` da QEMU (DEBT-0002).
pub const QEMU_VIRT_PL011: u64 = 0x0900_0000;
pub const QEMU_VIRT_GICD: u64 = 0x0800_0000;
pub const QEMU_VIRT_GICR: u64 = 0x080A_0000;
