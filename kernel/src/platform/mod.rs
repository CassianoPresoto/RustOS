//! Escolha dos adaptadores por ISA. Cada módulo expõe a mesma superfície:
//! `ARCH`, `TIMER_HZ`, `PlatformConsole`, `cpu()`, `console()`, `init_interrupts()`,
//! `irq_source()`, `timer_tick()`, `serial_irq_done()`, `keyboard_event()`,
//! `now_ns()`, `protections()` e `trigger()`. O contrato de cada uma está no SDD 02.

#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(target_arch = "x86_64")]
mod x86_64;

#[cfg(target_arch = "aarch64")]
pub use aarch64::{
    ARCH, PlatformConsole, TIMER_HZ, console, cpu, init_interrupts, irq_source, keyboard_event,
    now_ns, protections, serial_irq_done, timer_tick, trigger,
};
#[cfg(target_arch = "x86_64")]
pub use x86_64::{
    ARCH, PlatformConsole, TIMER_HZ, console, cpu, init_interrupts, irq_source, keyboard_event,
    now_ns, protections, serial_irq_done, timer_tick, trigger,
};

/// Por que a plataforma não ligou as interrupções.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code, reason = "BASE: os adaptadores de cada ISA constroem")]
pub enum PlatformError {
    /// Esta ISA ainda não tem o caminho de interrupção.
    Unsupported,
    Map(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] rustos_hal::MapError),
    OutOfMemory,
    /// A CPU não tem um recurso de que o caminho depende (ex.: APIC).
    MissingFeature(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] &'static str),
}

/// Uma proteção de hardware e se ficou ligada (guidelines 09).
#[derive(Clone, Copy, Debug)]
pub struct Protection {
    pub name: &'static str,
    pub enabled: bool,
}
