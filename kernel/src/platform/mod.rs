//! Escolha dos adaptadores por ISA. Cada módulo expõe a mesma superfície, listada
//! nos `pub use` abaixo: console, CPU, interrupções, relógio, entropia, proteções,
//! espaço de endereçamento, troca de pilha, troca de contexto, estado de FPU/SIMD e
//! timer de um tiro. O contrato de cada uma está no SDD 02 e no SDD 15.

#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(target_arch = "x86_64")]
mod x86_64;

#[cfg(target_arch = "aarch64")]
pub use aarch64::{
    ARCH, Context, CpuPrefix, EarlyScratch, FPU_AREA_ALIGN, PlatformConsole, PlatformSpace,
    SecondaryResources, TIMER_HZ, active_space, arm_timer, call_on_stack, console, copy_from_user,
    copy_to_user, cpu, cpu_local, cycles, cycles_to_ns, early_console, early_init,
    early_serial_map, early_serial_switch, enter_user, fpu_area_bytes, fpu_init, fpu_init_area,
    fpu_restore, fpu_save, harden, hardware_entropy, has_keyboard, init_interrupts, ipi,
    ipi_target, irq_source, kernel_half_in_sync, kernel_root, keyboard_event, local_hw_id,
    new_space, new_user_space, now_ns, prepare_secondary, protections, sched_clock_ns,
    secondary_early, secondary_init, serial_irq_done, switch, switch_space, trigger, user_cpu_init,
    user_init,
};
#[cfg(target_arch = "x86_64")]
pub use x86_64::{
    ARCH, Context, CpuPrefix, EarlyScratch, FPU_AREA_ALIGN, PlatformConsole, PlatformSpace,
    SecondaryResources, TIMER_HZ, active_space, arm_timer, call_on_stack, console, copy_from_user,
    copy_to_user, cpu, cpu_local, cycles, cycles_to_ns, early_console, early_init,
    early_serial_map, early_serial_switch, enter_user, fpu_area_bytes, fpu_init, fpu_init_area,
    fpu_restore, fpu_save, harden, hardware_entropy, has_keyboard, init_interrupts, ipi,
    ipi_target, irq_source, kernel_half_in_sync, kernel_root, keyboard_event, local_hw_id,
    new_space, new_user_space, now_ns, prepare_secondary, protections, sched_clock_ns,
    secondary_early, secondary_init, serial_irq_done, switch, switch_space, trigger, user_cpu_init,
    user_init,
};

#[cfg(all(feature = "selftest", target_arch = "aarch64"))]
pub use aarch64::{HAS_FAST_RETURN, set_fast_return, set_stack_clean, user_blob};
#[cfg(all(feature = "selftest", target_arch = "x86_64"))]
pub use x86_64::{HAS_FAST_RETURN, set_fast_return, set_stack_clean, user_blob};

#[cfg(all(feature = "selftest", target_arch = "aarch64"))]
pub use aarch64::{clock_error_ppm, fpu_read_probe, fpu_write_probe, timer_mode};
#[cfg(all(feature = "selftest", target_arch = "x86_64"))]
pub use x86_64::{clock_error_ppm, fpu_read_probe, fpu_write_probe, timer_mode};

#[cfg(target_arch = "aarch64")]
pub use aarch64::zero_aligned;
#[cfg(target_arch = "x86_64")]
pub use x86_64::zero_aligned;

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

/// O que `fpu_init` ligou, para o relatório do boot.
#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "lido só pelo modo de teste")
)]
#[derive(Clone, Copy, Debug)]
pub struct FpuSummary {
    pub mode: &'static str,
    pub bytes: usize,
    pub components: &'static str,
}
