//! Proteções de hardware que o CPUID oferece: WP, NX, SMEP, SMAP e UMIP.

use crate::sysreg::{
    CR0_WP, CR4_SMAP, CR4_SMEP, CR4_UMIP, EFER_NXE, MSR_EFER, features, rdmsr, read_cr0, read_cr4,
    write_cr0, write_cr4, wrmsr,
};

/// O que ficou ligado depois de `enable`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "um bit de registrador por campo"
)]
pub struct Protections {
    pub wp: bool,
    pub nx: bool,
    pub smep: bool,
    pub smap: bool,
    pub umip: bool,
}

/// Liga o que a CPU anuncia e devolve o estado lido de volta dos registradores.
///
/// # Safety
/// Ring 0. O kernel não lê nem executa memória de usuário: SMEP e SMAP valem desde já.
pub unsafe fn enable() -> Protections {
    let features = features();
    // SAFETY: ring 0, pelo contrato; WP só endurece escrita em página somente leitura.
    unsafe { write_cr0(read_cr0() | CR0_WP) };
    let mut cr4 = read_cr4();
    if features.smep {
        cr4 |= CR4_SMEP;
    }
    if features.smap {
        cr4 |= CR4_SMAP;
    }
    crate::usercopy::set_smap(features.smap);
    if features.umip {
        cr4 |= CR4_UMIP;
    }
    // SAFETY: só liga bits que o CPUID anunciou.
    unsafe { write_cr4(cr4) };
    // SAFETY: `EFER` existe em todo x86_64.
    let efer = unsafe { rdmsr(MSR_EFER) };
    if features.nx && efer & EFER_NXE == 0 {
        // SAFETY: o CPUID anuncia NX; só liga o bit NXE.
        unsafe { wrmsr(MSR_EFER, efer | EFER_NXE) };
    }
    // SAFETY: `EFER` existe em todo x86_64.
    let efer = unsafe { rdmsr(MSR_EFER) };
    let (cr0, cr4) = (read_cr0(), read_cr4());
    Protections {
        wp: cr0 & CR0_WP != 0,
        nx: efer & EFER_NXE != 0,
        smep: cr4 & CR4_SMEP != 0,
        smap: cr4 & CR4_SMAP != 0,
        umip: cr4 & CR4_UMIP != 0,
    }
}
