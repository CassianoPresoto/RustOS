//! Adaptador da ISA x86_64: o único lugar com assembly de x86.
//! O que tem `asm!` só compila para o alvo bare-metal; o resto é testável no host.

#![cfg_attr(not(test), no_std)]

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod cpu;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod port;

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use cpu::X86Cpu;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use port::PortBus;

/// Base da COM1 no PC.
pub const COM1: u16 = 0x3F8;
