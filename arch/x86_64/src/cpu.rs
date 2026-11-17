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

    fn save_and_disable_interrupts(&self) -> bool {
        let flags: u64;
        // SAFETY: `new` exige ring 0; lê RFLAGS pela pilha e desliga IF.
        unsafe { asm!("pushfq", "pop {}", "cli", out(reg) flags, options(preserves_flags)) };
        flags & RFLAGS_IF != 0
    }

    fn restore_interrupts(&self, were_enabled: bool) {
        if were_enabled {
            // SAFETY: `new` exige ring 0. Sem `nomem`: barreira para o compilador.
            unsafe { asm!("sti", options(nostack, preserves_flags)) };
        }
    }

    fn enable_interrupts_and_wait(&self) {
        // SAFETY: ring 0. O `sti` só vale depois da instrução seguinte: o `hlt`
        // começa antes de qualquer interrupção ser entregue, e nenhuma se perde.
        unsafe { asm!("sti", "hlt", options(nostack, preserves_flags)) };
    }
}

/// Bit IF do RFLAGS: interrupções mascaráveis ligadas.
const RFLAGS_IF: u64 = 1 << 9;
