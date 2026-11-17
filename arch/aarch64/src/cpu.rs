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

    fn save_and_disable_interrupts(&self) -> bool {
        let daif: u64;
        // SAFETY: `new` exige EL1+; lê DAIF e mascara IRQ (bit I).
        unsafe {
            asm!("mrs {}, daif", "msr daifset, #2", out(reg) daif, options(nostack, preserves_flags));
        };
        daif & DAIF_I == 0
    }

    fn restore_interrupts(&self, were_enabled: bool) {
        if were_enabled {
            // SAFETY: `new` exige EL1+. Sem `nomem`: barreira para o compilador.
            unsafe { asm!("msr daifclr, #2", options(nostack, preserves_flags)) };
        }
    }

    fn enable_interrupts_and_wait(&self) {
        // SAFETY: EL1+. `wfi` acorda com IRQ pendente mesmo mascarada; só então
        // o bit I cai e a IRQ é atendida. Nenhuma se perde entre checar e dormir.
        unsafe {
            asm!(
                "wfi",
                "msr daifclr, #2",
                "isb",
                options(nostack, preserves_flags)
            );
        }
    }
}

/// Bit I do DAIF: IRQ mascarada.
const DAIF_I: u64 = 1 << 7;
