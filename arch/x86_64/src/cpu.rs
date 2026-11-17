//! `hal::Cpu` sobre as instruções privilegiadas do x86_64.

use core::arch::asm;

use rustos_hal::Cpu;

#[derive(Clone, Copy, Debug)]
pub struct X86Cpu {
    _ring0: (),
}

impl X86Cpu {
    /// # Safety
    /// Só em ring 0: `cli` e `hlt` fora dele geram #GP.
    pub const unsafe fn new() -> Self {
        Self { _ring0: () }
    }
}

impl Cpu for X86Cpu {
    fn disable_interrupts(&self) {
        // SAFETY: `new` exige ring 0. Sem `nomem`: serve de barreira para o compilador.
        unsafe { asm!("cli", options(nostack, preserves_flags)) };
    }

    fn wait_for_interrupt(&self) {
        // SAFETY: `new` exige ring 0; `hlt` só suspende até a próxima interrupção.
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}
