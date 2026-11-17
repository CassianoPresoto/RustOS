//! Bits dos registradores de identificação e de controle que o adaptador consulta.

/// `ID_AA64MMFR1_EL1.PAN` (bits 23:20): diferente de zero, a CPU tem `PAN`.
pub const fn pan_supported(id_aa64mmfr1: u64) -> bool {
    (id_aa64mmfr1 >> 20) & 0xf != 0
}

/// `ID_AA64ISAR0_EL1.RNDR` (bits 63:60): diferente de zero, a CPU tem `RNDR` e `RNDRRS`
/// (`arch/arm64/tools/sysreg` do Linux: `0b0001 IMP`).
pub const fn rndr_supported(id_aa64isar0: u64) -> bool {
    (id_aa64isar0 >> 60) & 0xf != 0
}

/// `SCTLR_EL1.SPAN`: com zero, a exceção para o EL1 liga `PSTATE.PAN`.
pub const SCTLR_SPAN: u64 = 1 << 23;

/// `PSTATE.PAN` como o registrador `PAN` o devolve.
pub const PSTATE_PAN: u64 = 1 << 22;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rndr_field_is_read_from_bits_63_to_60() {
        assert!(!rndr_supported(0));
        assert!(rndr_supported(1 << 60));
        assert!(!rndr_supported(u64::MAX >> 4));
    }

    #[test]
    fn pan_field_is_read_from_bits_23_to_20() {
        assert!(!pan_supported(0));
        assert!(pan_supported(1 << 20));
        assert!(pan_supported(3 << 20));
        assert!(!pan_supported(0xf << 24 | 0xf_ffff));
    }
}
