//! `hal::HardwareEntropy` sobre `RNDRRS` (semeado de novo a cada leitura), com `RNDR` de reserva.

use core::arch::asm;

use rustos_hal::HardwareEntropy;
use rustos_hal::entropy::fill_from_words;

use crate::features::{ENTROPY_RNDR, ENTROPY_RNDRRS, entropy_source_name, rndr_supported};

/// Tentativas por palavra: a leitura sinaliza falha com `NZCV = 0b0100` e pede nova tentativa.
const ATTEMPTS: u32 = 16;

#[derive(Clone, Copy, Debug)]
pub struct Aarch64Entropy {
    _announced: (),
}

impl Aarch64Entropy {
    /// `None` se `ID_AA64ISAR0_EL1.RNDR` for zero.
    pub fn detect() -> Option<Self> {
        let isar0: u64;
        // SAFETY: leitura de registrador de identificação em EL1, sem efeito colateral.
        unsafe {
            asm!("mrs {}, id_aa64isar0_el1", out(reg) isar0, options(nomem, nostack, preserves_flags));
        };
        rndr_supported(isar0).then_some(Self { _announced: () })
    }
}

impl HardwareEntropy for Aarch64Entropy {
    fn fill(&self, out: &mut [u8]) -> Option<&'static str> {
        let next = || {
            rndrrs()
                .map(|word| (word, ENTROPY_RNDRRS))
                .or_else(|| rndr().map(|word| (word, ENTROPY_RNDR)))
        };
        fill_from_words(out, ATTEMPTS, next).map(entropy_source_name)
    }
}

/// `S3_3_C2_C4_1`. Sucesso deixa `NZCV = 0b0000`; falha deixa `0b0100`, e `Z` vira `eq`.
fn rndrrs() -> Option<u64> {
    let (word, ok): (u64, u64);
    // SAFETY: só se chega aqui com `RNDR` anunciado (`detect`); a leitura só escreve o registrador e `NZCV`.
    unsafe {
        asm!("mrs {}, s3_3_c2_c4_1", "cset {}, ne", out(reg) word, out(reg) ok, options(nomem, nostack));
    };
    (ok != 0).then_some(word)
}

/// `S3_3_C2_C4_0`, mesma convenção de `NZCV`.
fn rndr() -> Option<u64> {
    let (word, ok): (u64, u64);
    // SAFETY: só se chega aqui com `RNDR` anunciado (`detect`); a leitura só escreve o registrador e `NZCV`.
    unsafe {
        asm!("mrs {}, s3_3_c2_c4_0", "cset {}, ne", out(reg) word, out(reg) ok, options(nomem, nostack));
    };
    (ok != 0).then_some(word)
}
