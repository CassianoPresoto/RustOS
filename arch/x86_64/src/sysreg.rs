//! Registradores de sistema: CPUID, MSR, `CR0`/`CR4` e o TSC.

use core::arch::asm;
use core::arch::x86_64::{__cpuid_count, _rdtsc};

use crate::features::{Features, Leaves};

pub const MSR_EFER: u32 = 0xC000_0080;
/// `IA32_GS_BASE` e `IA32_KERNEL_GS_BASE` (`msr-index.h` do Linux).
pub const MSR_GS_BASE: u32 = 0xC000_0101;
pub const MSR_KERNEL_GS_BASE: u32 = 0xC000_0102;
pub const EFER_NXE: u64 = 1 << 11;
pub const CR0_MP: u64 = 1 << 1;
pub const CR0_EM: u64 = 1 << 2;
pub const CR0_TS: u64 = 1 << 3;
pub const CR0_NE: u64 = 1 << 5;
pub const CR0_WP: u64 = 1 << 16;
pub const CR4_TSD: u64 = 1 << 2;
pub const CR4_PGE: u64 = 1 << 7;
pub const CR4_OSFXSR: u64 = 1 << 9;
pub const CR4_OSXMMEXCPT: u64 = 1 << 10;
pub const CR4_OSXSAVE: u64 = 1 << 18;
/// `IA32_TSC_DEADLINE` (`msr-index.h` do Linux).
pub const MSR_TSC_DEADLINE: u32 = 0x6E0;
pub const CR4_UMIP: u64 = 1 << 11;
pub const CR4_SMEP: u64 = 1 << 20;
pub const CR4_SMAP: u64 = 1 << 21;

/// Lê o MSR `msr`.
///
/// # Safety
/// Ring 0, e o MSR existe nesta CPU: ler MSR inexistente gera #GP.
pub unsafe fn rdmsr(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    // SAFETY: o contrato da função garante ring 0 e MSR existente.
    unsafe {
        asm!("rdmsr", in("ecx") msr, out("eax") low, out("edx") high,
             options(nomem, nostack, preserves_flags));
    }
    u64::from(high) << 32 | u64::from(low)
}

/// Escreve `value` no MSR `msr`.
///
/// # Safety
/// Ring 0, o MSR existe e `value` é válido para ele; o efeito vale para a máquina toda.
#[allow(
    clippy::cast_possible_truncation,
    reason = "wrmsr recebe as duas metades"
)]
pub unsafe fn wrmsr(msr: u32, value: u64) {
    // SAFETY: o contrato da função garante ring 0, MSR existente e valor válido.
    unsafe {
        asm!("wrmsr", in("ecx") msr, in("eax") value as u32, in("edx") (value >> 32) as u32,
             options(nostack, preserves_flags));
    }
}

pub fn read_cr0() -> u64 {
    let value: u64;
    // SAFETY: ler `CR0` não tem efeito; o kernel roda em ring 0.
    unsafe { asm!("mov {}, cr0", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

/// # Safety
/// Ring 0, e `value` mantém válido o modo de execução (paginação, WP) em uso.
pub unsafe fn write_cr0(value: u64) {
    // SAFETY: o contrato da função garante ring 0 e valor coerente com o modo atual.
    unsafe { asm!("mov cr0, {}", in(reg) value, options(nostack, preserves_flags)) };
}

pub fn read_cr4() -> u64 {
    let value: u64;
    // SAFETY: ler `CR4` não tem efeito; o kernel roda em ring 0.
    unsafe { asm!("mov {}, cr4", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

/// # Safety
/// Ring 0, e só liga bits que o CPUID anuncia; bit reservado gera #GP.
pub unsafe fn write_cr4(value: u64) {
    // SAFETY: o contrato da função garante ring 0 e bits suportados.
    unsafe { asm!("mov cr4, {}", in(reg) value, options(nostack, preserves_flags)) };
}

/// Escreve `XCR0`, que escolhe os componentes de estado que XSAVE e XRSTOR tratam.
///
/// # Safety
/// Ring 0 com `OSXSAVE` ligado; `value` só tem bits de `CPUID.0DH.0` e o x87.
#[allow(
    clippy::cast_possible_truncation,
    reason = "xsetbv recebe as duas metades"
)]
pub unsafe fn write_xcr0(value: u64) {
    // SAFETY: o contrato da função garante `OSXSAVE`, ring 0 e um valor válido.
    unsafe {
        asm!("xsetbv", in("ecx") 0u32, in("eax") value as u32, in("edx") (value >> 32) as u32,
             options(nomem, nostack, preserves_flags));
    }
}

/// `CPUID.0DH` com a sub-folha pedida: `(eax, ebx, ecx, edx)`.
pub fn cpuid_xstate(subleaf: u32) -> (u32, u32, u32, u32) {
    let leaf = __cpuid_count(0xD, subleaf);
    (leaf.eax, leaf.ebx, leaf.ecx, leaf.edx)
}

pub fn rdtsc() -> u64 {
    // SAFETY: `rdtsc` é legal em ring 0 e não toca memória.
    unsafe { _rdtsc() }
}

/// O que o CPUID anuncia sobre APIC, TSC, NX, SMEP, SMAP e UMIP.
pub fn features() -> Features {
    let basic = __cpuid_count(0, 0);
    let leaf1 = __cpuid_count(1, 0);
    let leaf7 = __cpuid_count(7, 0);
    let ext = __cpuid_count(0x8000_0001, 0);
    Features::from_leaves(Leaves {
        max_basic: basic.eax,
        leaf1_ecx: leaf1.ecx,
        leaf1_edx: leaf1.edx,
        leaf7_ebx: leaf7.ebx,
        leaf7_ecx: leaf7.ecx,
        ext_edx: ext.edx,
    })
}
