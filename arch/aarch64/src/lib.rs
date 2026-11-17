//! Adaptador da ISA aarch64: o único lugar com assembly de ARM.
//! O que tem `asm!` só compila para o alvo bare-metal; o resto é testável no host.

#![cfg_attr(not(test), no_std)]

pub mod descriptor;
pub mod walk;

#[cfg(all(target_arch = "aarch64", target_os = "none"))]
mod cpu;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
mod mmio;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
mod mmu;

#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub use cpu::Aarch64Cpu;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub use mmio::MmioBus;
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub use mmu::Aarch64PageMapper;

/// PL011 da máquina `virt` da QEMU (DEBT-0002).
pub const QEMU_VIRT_PL011: u64 = 0x0900_0000;
