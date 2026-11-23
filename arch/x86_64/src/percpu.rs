//! `hal::CpuLocal` sobre o `GS`: o bloco por CPU mora em `IA32_GS_BASE`, e `gs:[0]`
//! devolve o endereço dele sem `rdmsr`.

use core::arch::asm;
use core::ptr;

use rustos_hal::CpuLocal;

use crate::prefix::CpuPrefix;

use crate::sysreg::{MSR_GS_BASE, MSR_KERNEL_GS_BASE, wrmsr};

#[derive(Clone, Copy, Debug)]
pub struct X86CpuLocal {
    _ring0: (),
}

impl X86CpuLocal {
    /// # Safety
    /// Só em ring 0: `wrmsr` fora dele gera #GP.
    pub const unsafe fn new() -> Self {
        Self { _ring0: () }
    }
}

impl CpuLocal for X86CpuLocal {
    /// `KERNEL_GS_BASE` fica em zero: é o `GS` do usuário que o `swapgs` troca (fase 4).
    fn install(&self, block: usize) {
        // SAFETY: `new` exige ring 0; os dois MSRs existem em todo x86_64.
        unsafe { wrmsr(MSR_GS_BASE, block as u64) };
        // SAFETY: idem.
        unsafe { wrmsr(MSR_KERNEL_GS_BASE, 0) };
    }

    #[allow(
        clippy::inline_always,
        reason = "no caminho da syscall nula: um quadro a mais passa da janela que o `arch/` repinta"
    )]
    #[inline(always)]
    fn block(&self) -> usize {
        let block: usize;
        // SAFETY: o contrato de `install` põe o próprio endereço em `gs:[0]`. Sem `nomem`
        // nem `pure`: o compilador não pode reaproveitar a leitura depois de uma troca de CPU.
        unsafe {
            asm!("mov {}, gs:[0]", out(reg) block, options(nostack, preserves_flags, readonly));
        }
        block
    }
}

/// O começo do bloco desta CPU: o `PerCpu` do kernel o embute como primeiro campo.
pub(crate) fn prefix() -> &'static CpuPrefix {
    let block: usize;
    // SAFETY: `gs:[0]` é o endereço do bloco, posto por `install`; leitura sem efeito colateral.
    unsafe { asm!("mov {}, gs:[0]", out(reg) block, options(nostack, preserves_flags, readonly)) };
    // SAFETY: o bloco existe para sempre e começa por um `CpuPrefix` (contrato do kernel).
    unsafe { &*ptr::with_exposed_provenance::<CpuPrefix>(block) }
}
