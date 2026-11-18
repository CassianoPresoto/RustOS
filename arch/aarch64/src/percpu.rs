//! `hal::CpuLocal` sobre o `TPIDR_EL1`: o endereço do bloco por CPU mora nele.

use core::arch::asm;
use core::ptr;

use rustos_hal::CpuLocal;

use crate::prefix::CpuPrefix;

#[derive(Clone, Copy, Debug)]
pub struct Aarch64CpuLocal {
    _privileged: (),
}

impl Aarch64CpuLocal {
    /// # Safety
    /// Só em EL1 ou acima: `TPIDR_EL1` é inacessível em EL0.
    pub const unsafe fn new() -> Self {
        Self { _privileged: () }
    }
}

impl CpuLocal for Aarch64CpuLocal {
    fn install(&self, block: usize) {
        // SAFETY: `new` exige EL1+; o registrador é de software e não tem efeito colateral.
        unsafe { asm!("msr tpidr_el1, {}", in(reg) block, options(nostack, preserves_flags)) };
    }

    fn block(&self) -> usize {
        let block: usize;
        // SAFETY: `new` exige EL1+. Sem `nomem` nem `pure`: o compilador não pode reaproveitar
        // a leitura depois de uma troca de CPU.
        unsafe {
            asm!("mrs {}, tpidr_el1", out(reg) block, options(nostack, preserves_flags, readonly));
        }
        block
    }
}

/// O começo do bloco desta CPU: o `PerCpu` do kernel o embute como primeiro campo.
pub(crate) fn prefix() -> &'static CpuPrefix {
    let block: usize;
    // SAFETY: EL1; `TPIDR_EL1` é o endereço do bloco, posto por `install`.
    unsafe {
        asm!("mrs {}, tpidr_el1", out(reg) block, options(nostack, preserves_flags, readonly));
    }
    // SAFETY: o bloco existe para sempre e começa por um `CpuPrefix` (contrato do kernel).
    unsafe { &*ptr::with_exposed_provenance::<CpuPrefix>(block) }
}
