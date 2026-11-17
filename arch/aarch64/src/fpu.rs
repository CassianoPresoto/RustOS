//! Estado de FP/SIMD por tarefa: liga o acesso em EL1 e EL0 e salva V0–V31,
//! FPSR e FPCR numa área que o kernel aloca. O kernel segue soft-float.
//! `CPACR_EL1.FPEN` como em `cpu_enable_fpsimd` e o layout de
//! `struct user_fpsimd_state` em `arch/arm64/` do Linux.

use core::arch::asm;

/// Bytes da área: 32 registradores de 128 bits mais FPSR e FPCR de 32 bits, e 8 de reserva.
pub const AREA_BYTES: usize = 528;
/// `stp q` e `ldp q` não exigem mais que 16 de alinhamento.
pub const AREA_ALIGN: usize = 16;

/// `CPACR_EL1.FPEN` (bits 21:20) em `0b11`: sem trap de FP/SIMD em EL1 nem em EL0.
const CPACR_FPEN_ALL: u64 = 0b11 << 20;

/// Libera FP/SIMD em EL1 e EL0 (a CPU sai do Limine com `CPACR_EL1` zerado).
///
/// # Safety
/// Em EL1, uma vez por CPU.
pub unsafe fn init() {
    let cpacr: u64;
    // SAFETY: leitura de registrador de sistema em EL1.
    unsafe { asm!("mrs {}, cpacr_el1", out(reg) cpacr, options(nomem, nostack, preserves_flags)) };
    // SAFETY: só liga `FPEN`; o `isb` publica a mudança antes do primeiro acesso a `q0`.
    unsafe {
        asm!(
            "msr cpacr_el1, {}", "isb",
            in(reg) cpacr | CPACR_FPEN_ALL,
            options(nostack, preserves_flags),
        );
    }
}

/// Salva V0–V31, FPSR e FPCR em `area`.
///
/// # Safety
/// `init` rodou; `area` tem `AREA_BYTES` bytes gravaveis, alinhados a `AREA_ALIGN`.
pub unsafe fn save(area: *mut u8) {
    // SAFETY: contrato da função.
    unsafe {
        asm!(
            ".arch_extension fp", ".arch_extension simd",
            "stp q0, q1, [{a}]", "stp q2, q3, [{a}, #32]",
            "stp q4, q5, [{a}, #64]", "stp q6, q7, [{a}, #96]",
            "stp q8, q9, [{a}, #128]", "stp q10, q11, [{a}, #160]",
            "stp q12, q13, [{a}, #192]", "stp q14, q15, [{a}, #224]",
            "stp q16, q17, [{a}, #256]", "stp q18, q19, [{a}, #288]",
            "stp q20, q21, [{a}, #320]", "stp q22, q23, [{a}, #352]",
            "stp q24, q25, [{a}, #384]", "stp q26, q27, [{a}, #416]",
            "stp q28, q29, [{a}, #448]", "stp q30, q31, [{a}, #480]",
            "mrs {t}, fpsr", "str {t:w}, [{a}, #512]",
            "mrs {t}, fpcr", "str {t:w}, [{a}, #516]",
            a = in(reg) area, t = out(reg) _,
            options(nostack, preserves_flags),
        );
    }
}

/// Carrega V0–V31, FPSR e FPCR de `area`.
///
/// # Safety
/// `init` rodou; `area` tem `AREA_BYTES` bytes alinhados e veio de `save` ou está zerada.
pub unsafe fn restore(area: *const u8) {
    // SAFETY: contrato da função.
    unsafe {
        asm!(
            ".arch_extension fp", ".arch_extension simd",
            "ldp q0, q1, [{a}]", "ldp q2, q3, [{a}, #32]",
            "ldp q4, q5, [{a}, #64]", "ldp q6, q7, [{a}, #96]",
            "ldp q8, q9, [{a}, #128]", "ldp q10, q11, [{a}, #160]",
            "ldp q12, q13, [{a}, #192]", "ldp q14, q15, [{a}, #224]",
            "ldp q16, q17, [{a}, #256]", "ldp q18, q19, [{a}, #288]",
            "ldp q20, q21, [{a}, #320]", "ldp q22, q23, [{a}, #352]",
            "ldp q24, q25, [{a}, #384]", "ldp q26, q27, [{a}, #416]",
            "ldp q28, q29, [{a}, #448]", "ldp q30, q31, [{a}, #480]",
            "ldr {t:w}, [{a}, #512]", "msr fpsr, {t}",
            "ldr {t:w}, [{a}, #516]", "msr fpcr, {t}",
            a = in(reg) area, t = out(reg) _,
            options(nostack, preserves_flags),
        );
    }
}

/// Escreve `pattern` em `v0`. Só o teste de troca de contexto usa: o compilador
/// não aloca `v0` (alvo soft-float), então o registrador é de quem o salva.
pub fn write_probe(pattern: &[u8; 16]) {
    // SAFETY: lê 16 bytes de `pattern`; `v0` não é usado pelo código soft-float.
    unsafe {
        asm!(
            ".arch_extension fp", ".arch_extension simd",
            "ldr q0, [{p}]",
            p = in(reg) pattern.as_ptr(),
            options(nostack, preserves_flags),
        );
    }
}

/// Lê `v0`.
pub fn read_probe() -> [u8; 16] {
    let mut out = [0_u8; 16];
    // SAFETY: grava 16 bytes em `out`; `v0` não é usado pelo código soft-float.
    unsafe {
        asm!(
            ".arch_extension fp", ".arch_extension simd",
            "str q0, [{p}]",
            p = in(reg) out.as_mut_ptr(),
            options(nostack, preserves_flags),
        );
    }
    out
}
