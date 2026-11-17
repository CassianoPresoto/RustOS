//! Proteções de hardware que o kernel liga e confere no próprio registrador.

use core::arch::asm;

use crate::features::{PSTATE_PAN, SCTLR_SPAN, pan_supported};

/// `msr pan, #1`: o assembler não conhece `PAN` sem `+pan`, então vai codificado.
const MSR_PAN_1: u32 = 0xd500_419f;

/// A CPU anuncia `PAN` em `ID_AA64MMFR1_EL1`.
pub fn pan_available() -> bool {
    let mmfr1: u64;
    // SAFETY: leitura de registrador de identificação, sem efeito colateral.
    unsafe {
        asm!("mrs {}, id_aa64mmfr1_el1", out(reg) mmfr1, options(nomem, nostack, preserves_flags));
    };
    pan_supported(mmfr1)
}

/// Liga `PAN` agora e na entrada de toda exceção; devolve o `PSTATE.PAN` lido.
/// # Safety
/// Em EL1 e com `pan_available()` verdadeiro.
pub unsafe fn enable_pan() -> bool {
    let mut sctlr: u64;
    // SAFETY: EL1, pelo contrato da função.
    unsafe { asm!("mrs {}, sctlr_el1", out(reg) sctlr, options(nomem, nostack, preserves_flags)) };
    sctlr &= !SCTLR_SPAN;
    // SAFETY: só o bit `SPAN` muda; `isb` o torna efetivo para as próximas exceções.
    unsafe { asm!("msr sctlr_el1, {}", "isb", in(reg) sctlr, options(nostack, preserves_flags)) };
    // SAFETY: a CPU tem `PAN`, pelo contrato; a instrução só liga o bit de estado.
    unsafe { asm!(".inst {}", const MSR_PAN_1, options(nomem, nostack, preserves_flags)) };
    pan_active()
}

/// `PSTATE.PAN`, lido do registrador `PAN` (`S3_0_C4_C2_3`).
pub fn pan_active() -> bool {
    let pan: u64;
    // SAFETY: só roda com `PAN` disponível; leitura de estado, sem efeito colateral.
    unsafe { asm!("mrs {}, S3_0_C4_C2_3", out(reg) pan, options(nomem, nostack, preserves_flags)) };
    pan & PSTATE_PAN != 0
}
