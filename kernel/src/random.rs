//! Gerador aleatório do kernel: semente do hardware, ou jitter do contador quando a
//! CPU não tem gerador, e o estado global com reposição por volume entregue.

use rustos_crypto::seed::{HARDWARE_SAMPLE_LEN, gather_jitter, hardware_output_plausible};
use rustos_crypto::{Rng, wipe};
use rustos_hal::HardwareEntropy;

use crate::platform;
use crate::sync::IrqLock;

static RNG: IrqLock<Option<Rng>> = IrqLock::new(None);

/// Bytes entregues antes de misturar semente nova; a conta está no SDD 07.
const RESEED_BYTES: u64 = 4 << 20;
/// Leituras do contador por mistura de jitter.
const JITTER_ROUNDS: usize = 512;
/// Marcos do boot que entram na mistura: o que couber no buffer.
const MARKS_MAX: usize = 16;

/// De onde veio a semente.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Hardware(&'static str),
    /// Só jitter do contador e marcos do boot: fraca (DEBT-0019).
    Jitter,
}

/// Semeia o gerador global. `marks` são os marcos impredizíveis do boot (contadores,
/// tempos do bootloader): entram por `reseed` junto com o jitter, qualquer que seja a fonte.
pub fn init(marks: &[u64]) -> Source {
    let mut slot = RNG.lock();
    replenish(start(&mut slot), marks)
}

/// Põe no `slot` o gerador com a chave de `Rng::new(&[])`, semeada no lugar: a chave não
/// passa por cópia na pilha.
fn start(slot: &mut Option<Rng>) -> &mut Rng {
    let rng = slot.insert(Rng::unseeded());
    rng.reseed(&[]);
    rng
}

/// Um número de 64 bits; `None` antes de `init`. Repõe a semente a cada `RESEED_BYTES`.
pub fn next_u64() -> Option<u64> {
    let mut slot = RNG.lock();
    let rng = slot.as_mut()?;
    if rng.bytes_since_seed() >= RESEED_BYTES {
        replenish(rng, &[platform::cycles()]);
    }
    Some(rng.next_u64())
}

/// Mistura no gerador a semente de hardware (se a CPU tem e a saída parece viva),
/// o jitter do contador e os marcos. A semente em pilha é zerada ao sair.
fn replenish(rng: &mut Rng, marks: &[u64]) -> Source {
    let mut seed = [0_u8; HARDWARE_SAMPLE_LEN];
    let from_hardware = platform::hardware_entropy()
        .and_then(|hardware| hardware.fill(&mut seed))
        .filter(|_| hardware_output_plausible(&seed));
    let source = match from_hardware {
        Some(name) => {
            rng.reseed(&seed);
            Source::Hardware(name)
        }
        None => Source::Jitter,
    };
    wipe(&mut seed);
    let mut jitter = gather_jitter(platform::cycles, JITTER_ROUNDS);
    rng.reseed(&jitter);
    wipe(&mut jitter);
    let mut bytes = [0_u8; MARKS_MAX * 8];
    for (chunk, mark) in bytes.chunks_mut(8).zip(marks) {
        chunk.copy_from_slice(&mark.to_le_bytes());
    }
    rng.reseed(&bytes[..marks.len().min(MARKS_MAX) * 8]);
    wipe(&mut bytes);
    source
}

#[cfg(feature = "selftest")]
mod selftest;
#[cfg(feature = "selftest")]
pub use selftest::{PLANTED, Residue, benchmark, stack_residue};
