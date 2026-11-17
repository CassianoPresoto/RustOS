//! `hal::HardwareEntropy` sobre `RDSEED`, com `RDRAND` de reserva.

use core::arch::asm;

use rustos_hal::HardwareEntropy;
use rustos_hal::entropy::fill_from_words;

use crate::features::{ENTROPY_RDRAND, ENTROPY_RDSEED, Features, entropy_source_name};

/// Tentativas por palavra: `RDSEED` falha com frequência sob carga, `RDRAND` quase nunca
/// (o Linux repete `RDRAND` 10 vezes, `RDRAND_RETRY_LOOPS`).
const RDSEED_ATTEMPTS: u32 = 64;
const RDRAND_ATTEMPTS: u32 = 10;

#[derive(Clone, Copy, Debug)]
pub struct X86Entropy {
    rdseed: bool,
    rdrand: bool,
}

impl X86Entropy {
    /// `None` se o CPUID não anuncia nenhuma das duas instruções.
    pub const fn detect(features: Features) -> Option<Self> {
        if features.rdseed || features.rdrand {
            Some(Self {
                rdseed: features.rdseed,
                rdrand: features.rdrand,
            })
        } else {
            None
        }
    }

    fn word(self) -> Option<(u64, u8)> {
        if self.rdseed {
            for _ in 0..RDSEED_ATTEMPTS {
                if let Some(word) = rdseed() {
                    return Some((word, ENTROPY_RDSEED));
                }
                core::hint::spin_loop();
            }
        }
        if self.rdrand {
            return (0..RDRAND_ATTEMPTS).find_map(|_| rdrand().map(|word| (word, ENTROPY_RDRAND)));
        }
        None
    }
}

impl HardwareEntropy for X86Entropy {
    fn fill(&self, out: &mut [u8]) -> Option<&'static str> {
        fill_from_words(out, 1, || self.word()).map(entropy_source_name)
    }
}

/// `CF = 1` é sucesso; `CF = 0` é "sem entropia agora, tente de novo".
fn rdseed() -> Option<u64> {
    let (word, ok): (u64, u8);
    // SAFETY: só se chega aqui com `rdseed` anunciado pelo CPUID (`detect`); a instrução
    // vale em qualquer anel, só escreve o registrador e a flag.
    unsafe {
        asm!("rdseed {}", "setc {}", out(reg) word, out(reg_byte) ok, options(nomem, nostack));
    };
    if ok != 0 { Some(word) } else { None }
}

fn rdrand() -> Option<u64> {
    let (word, ok): (u64, u8);
    // SAFETY: só se chega aqui com `rdrand` anunciado pelo CPUID (`detect`); a instrução
    // vale em qualquer anel, só escreve o registrador e a flag.
    unsafe {
        asm!("rdrand {}", "setc {}", out(reg) word, out(reg_byte) ok, options(nomem, nostack));
    };
    if ok != 0 { Some(word) } else { None }
}
