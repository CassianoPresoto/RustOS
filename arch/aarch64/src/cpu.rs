//! `hal::Cpu` sobre as instruções de sistema do aarch64.

use core::arch::asm;

use rustos_hal::Cpu;

#[derive(Clone, Copy, Debug)]
pub struct Aarch64Cpu {
    _privileged: (),
}

impl Aarch64Cpu {
    /// # Safety
    /// Só em EL1 ou EL2: em EL0 o acesso ao DAIF é proibido por padrão.
    pub const unsafe fn new() -> Self {
        Self { _privileged: () }
    }
}

impl Cpu for Aarch64Cpu {
    fn disable_interrupts(&self) {
        // SAFETY: `new` exige EL1+. Sem `nomem`: serve de barreira para o compilador.
        unsafe { asm!("msr daifset, #0xf", options(nostack, preserves_flags)) };
    }

    fn wait_for_interrupt(&self) {
        // SAFETY: `wfi` só suspende o núcleo até um evento ou interrupção.
        unsafe { asm!("wfi", options(nomem, nostack, preserves_flags)) };
    }
}
