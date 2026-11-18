//! Proteções de hardware que o kernel liga e confere no próprio registrador.

use core::arch::asm;

use crate::features::{PSTATE_PAN, SCTLR_SPAN, pan_supported};
use crate::mitigation::{MitigationFeatures, MitigationRegs};

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

/// Os registradores de identificação das mitigações, decodificados. `ID_AA64ISAR2_EL1` vai
/// codificado (`S3_0_C0_C6_2`): o assembler base não o conhece.
pub fn mitigation_features() -> MitigationFeatures {
    let (midr, pfr0, pfr1, mmfr1, isar2): (u64, u64, u64, u64, u64);
    // SAFETY: leitura de registradores de identificação em EL1, sem efeito colateral.
    unsafe {
        asm!(
            "mrs {midr}, midr_el1",
            "mrs {pfr0}, id_aa64pfr0_el1",
            "mrs {pfr1}, id_aa64pfr1_el1",
            "mrs {mmfr1}, id_aa64mmfr1_el1",
            "mrs {isar2}, S3_0_C0_C6_2",
            midr = out(reg) midr,
            pfr0 = out(reg) pfr0,
            pfr1 = out(reg) pfr1,
            mmfr1 = out(reg) mmfr1,
            isar2 = out(reg) isar2,
            options(nomem, nostack, preserves_flags),
        );
    }
    MitigationFeatures::from_regs(MitigationRegs {
        midr,
        id_aa64pfr0: pfr0,
        id_aa64pfr1: pfr1,
        id_aa64mmfr1: mmfr1,
        id_aa64isar2: isar2,
    })
}
