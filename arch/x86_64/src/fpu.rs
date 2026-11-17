//! Estado de FPU e SIMD por tarefa: liga SSE e XSAVE, escolhe o que salvar e
//! salva e restaura numa área que o kernel aloca. O kernel segue soft-float.
//! Bits de `CR0`/`CR4`/`XCR0` como em `fpu__init_cpu_generic` e
//! `fpu__init_cpu_xstate` (`arch/x86/kernel/fpu/` do Linux).

use core::arch::asm;
use core::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};

use crate::features::Features;
use crate::sysreg::{
    CR0_EM, CR0_MP, CR0_NE, CR0_TS, CR4_OSFXSR, CR4_OSXMMEXCPT, CR4_OSXSAVE, CR4_TSD, cpuid_xstate,
    read_cr0, read_cr4, write_cr0, write_cr4, write_xcr0,
};
use crate::xstate::{self, SaveMode, XCR0_X87};

pub use crate::xstate::AREA_ALIGN;

const MODE_XSAVEOPT: u8 = 0;
const MODE_XSAVE: u8 = 1;
const MODE_FXSAVE: u8 = 2;

static MODE: AtomicU8 = AtomicU8::new(MODE_FXSAVE);
static XCR0: AtomicU64 = AtomicU64::new(XCR0_X87);
static BYTES: AtomicUsize = AtomicUsize::new(512);

/// O que `init` ligou, para o relatório do boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FpuInfo {
    pub mode: SaveMode,
    pub bytes: usize,
    pub xcr0: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpuError {
    /// x86_64 sempre tem FXSR e SSE2; a CPU que não os anuncia não é suportada.
    NoSse,
}

/// Liga x87, SSE e, se houver, XSAVE com AVX e AVX-512; `CR4.TSD` fica limpo.
///
/// # Safety
/// Ring 0, uma vez por CPU, antes de qualquer `save` ou `restore`.
pub unsafe fn init(features: Features) -> Result<FpuInfo, FpuError> {
    if !features.fxsr || !features.sse {
        return Err(FpuError::NoSse);
    }
    let cr0 = (read_cr0() & !(CR0_TS | CR0_EM)) | CR0_MP | CR0_NE;
    // SAFETY: ring 0; limpar TS e EM e ligar MP e NE é o estado que o Linux usa para x87.
    unsafe { write_cr0(cr0) };
    let mut cr4 = (read_cr4() & !CR4_TSD) | CR4_OSFXSR | CR4_OSXMMEXCPT;
    if features.xsave {
        cr4 |= CR4_OSXSAVE;
    }
    // SAFETY: FXSR, SSE e XSAVE foram anunciados pelo CPUID; TSD limpo é sempre válido.
    unsafe { write_cr4(cr4) };
    // SAFETY: `fninit` só reinicia a FPU, sem efeito em memória.
    unsafe { asm!("fninit", options(nomem, nostack)) };

    let (mode, xcr0, bytes) = if features.xsave {
        let (supported_low, _, _, supported_high) = cpuid_xstate(0);
        let supported = u64::from(supported_high) << 32 | u64::from(supported_low);
        let wanted = xstate::wanted_xcr0(supported);
        // SAFETY: OSXSAVE está ligado acima e `wanted` só tem bits que o CPUID anuncia.
        unsafe { write_xcr0(wanted) };
        let (_, active_bytes, _, _) = cpuid_xstate(0);
        let (opt, _, _, _) = cpuid_xstate(1);
        (
            xstate::save_mode(true, opt & 1 != 0),
            wanted,
            xstate::area_bytes(active_bytes),
        )
    } else {
        (SaveMode::Fxsave, XCR0_X87, 512)
    };
    MODE.store(
        match mode {
            SaveMode::XsaveOpt => MODE_XSAVEOPT,
            SaveMode::Xsave => MODE_XSAVE,
            SaveMode::Fxsave => MODE_FXSAVE,
        },
        Ordering::Relaxed,
    );
    XCR0.store(xcr0, Ordering::Relaxed);
    BYTES.store(bytes, Ordering::Relaxed);
    Ok(FpuInfo { mode, bytes, xcr0 })
}

/// Bytes de cada área de salvamento; vale depois de `init`.
pub fn area_bytes() -> usize {
    BYTES.load(Ordering::Relaxed)
}

/// As duas metades do `XCR0` que o XSAVE e o XRSTOR recebem em `eax` e `edx`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "são as duas metades de 32 bits"
)]
fn mask_halves() -> (u32, u32) {
    let xcr0 = XCR0.load(Ordering::Relaxed);
    (xcr0 as u32, (xcr0 >> 32) as u32)
}

/// Salva o estado atual em `area`.
///
/// # Safety
/// `init` rodou; `area` tem `area_bytes()` bytes gravaveis, alinhados a `AREA_ALIGN`.
pub unsafe fn save(area: *mut u8) {
    let (low, high) = mask_halves();
    match MODE.load(Ordering::Relaxed) {
        // SAFETY: contrato da função; `init` ligou o XSAVE e o `XCR0` do pedido.
        MODE_XSAVEOPT => unsafe {
            asm!("xsaveopt64 [{a}]", a = in(reg) area, in("eax") low, in("edx") high,
                 options(nostack, preserves_flags));
        },
        // SAFETY: idem.
        MODE_XSAVE => unsafe {
            asm!("xsave64 [{a}]", a = in(reg) area, in("eax") low, in("edx") high,
                 options(nostack, preserves_flags));
        },
        // SAFETY: contrato da função; FXSR foi checado em `init`.
        _ => unsafe { asm!("fxsave64 [{a}]", a = in(reg) area, options(nostack, preserves_flags)) },
    }
}

/// Carrega o estado de `area`.
///
/// # Safety
/// `init` rodou; `area` veio de `save` ou de `xstate::init_area`, alinhada e com `area_bytes()`.
pub unsafe fn restore(area: *const u8) {
    let (low, high) = mask_halves();
    match MODE.load(Ordering::Relaxed) {
        // SAFETY: contrato da função.
        MODE_XSAVEOPT | MODE_XSAVE => unsafe {
            asm!("xrstor64 [{a}]", a = in(reg) area, in("eax") low, in("edx") high,
                 options(nostack, preserves_flags));
        },
        // SAFETY: contrato da função.
        _ => unsafe {
            asm!("fxrstor64 [{a}]", a = in(reg) area, options(nostack, preserves_flags));
        },
    }
}

/// Escreve `pattern` em `xmm0`. Só o teste de troca de contexto usa: o compilador
/// não aloca `xmm0` (alvo soft-float), então o registrador é de quem o salva.
pub fn write_probe(pattern: &[u8; 16]) {
    // SAFETY: lê 16 bytes de `pattern`; `xmm0` não é usado pelo código soft-float.
    unsafe {
        asm!("movdqu xmm0, [{p}]", p = in(reg) pattern.as_ptr(), options(nostack, preserves_flags));
    }
}

/// Lê `xmm0`.
pub fn read_probe() -> [u8; 16] {
    let mut out = [0_u8; 16];
    // SAFETY: grava 16 bytes em `out`; `xmm0` não é usado pelo código soft-float.
    unsafe {
        asm!("movdqu [{p}], xmm0", p = in(reg) out.as_mut_ptr(), options(nostack, preserves_flags));
    }
    out
}
