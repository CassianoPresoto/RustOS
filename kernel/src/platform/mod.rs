//! Escolha dos adaptadores por ISA. Cada módulo expõe a mesma superfície, listada
//! nos `pub use` abaixo: console, CPU, interrupções, relógio, entropia, proteções,
//! espaço de endereçamento e troca de pilha. O contrato de cada uma está no SDD 02.

#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(target_arch = "x86_64")]
mod x86_64;

#[cfg(target_arch = "aarch64")]
pub use aarch64::{
    ARCH, PlatformConsole, PlatformSpace, TIMER_HZ, call_on_stack, console, cpu, cycles, harden,
    hardware_entropy, has_keyboard, init_interrupts, irq_source, keyboard_event, new_space, now_ns,
    protections, serial_irq_done, timer_tick, trigger,
};
#[cfg(target_arch = "x86_64")]
pub use x86_64::{
    ARCH, PlatformConsole, PlatformSpace, TIMER_HZ, call_on_stack, console, cpu, cycles, harden,
    hardware_entropy, has_keyboard, init_interrupts, irq_source, keyboard_event, new_space, now_ns,
    protections, serial_irq_done, timer_tick, trigger,
};

/// Por que a plataforma não ligou as interrupções.
#[derive(Clone, Copy, Debug)]
pub enum PlatformError {
    Map(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] rustos_hal::MapError),
    /// A CPU não tem um recurso de que o caminho depende (ex.: APIC).
    MissingFeature(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] &'static str),
}

/// Uma proteção de hardware e se ficou ligada (guidelines 09).
#[derive(Clone, Copy, Debug)]
pub struct Protection {
    pub name: &'static str,
    pub enabled: bool,
}
