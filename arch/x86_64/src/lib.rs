//! Adaptador da ISA x86_64: o único lugar com assembly de x86.
//! O que tem `asm!` só compila para o alvo bare-metal; o resto é testável no host.

#![cfg_attr(not(test), no_std)]

pub mod clock;
pub mod fault;
pub mod features;
pub mod ioapic;
pub mod lapic;
pub mod pic;
pub mod pit;
pub mod pte;
pub mod segments;
pub mod xstate;

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod apic;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod context;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod cpu;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod entropy;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub mod fpu;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod mmio;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod mmu;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod percpu;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod port;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod protect;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod stack;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod sysreg;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
mod traps;

#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use apic::{
    ApicAccess, ApicMode, TimerMode, X86Ipi, arm_oneshot, arm_tsc_deadline, detect as detect_apic,
    enable_secondary as enable_secondary_apic, eoi, local_id as local_apic_id,
    publish as publish_apic, set_timer_mode,
};
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use context::{Context, switch};
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use cpu::X86Cpu;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use entropy::X86Entropy;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use mmio::MmioBus;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use mmu::{DirectMapTables, X86Mmu, active_root, require_features};
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use percpu::X86CpuLocal;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use port::PortBus;
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use protect::{Protections, enable as enable_protections};
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use stack::{Continuation, call_on_stack};
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use sysreg::{features as cpu_features, rdtsc};
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
pub use traps::{
    CpuTables, IstStacks, breakpoint, install as install_traps,
    install_early as install_early_traps, load_cpu as load_cpu_tables,
};

/// Base da COM1 no PC.
pub const COM1: u16 = 0x3F8;
/// Base das portas do PIT (canais em 0x40-0x42, comando em 0x43, porta B em 0x61).
pub const PIT_PORTS: u16 = 0x40;
/// Bases dos dois 8259 (máscaras em 0x21 e 0xA1).
pub const PIC_MASTER: u16 = 0x20;
pub const PIC_SLAVE: u16 = 0xA0;
