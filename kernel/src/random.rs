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
const BENCH_BLOCK: usize = 4096;
const BENCH_BLOCKS: usize = 32;

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
    let mut rng = Rng::new(&[]);
    let source = replenish(&mut rng, marks);
    *RNG.lock() = Some(rng);
    source
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
    let source = match platform::hardware_entropy() {
        Some(hardware) if hardware.fill(&mut seed) && hardware_output_plausible(&seed) => {
            rng.reseed(&seed);
            Source::Hardware(hardware.name())
        }
        _ => Source::Jitter,
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

/// Vazão do ChaCha20 e custo de uma reposição, medidos no kernel com o relógio da fase 1.
#[derive(Clone, Copy, Debug)]
pub struct Benchmark {
    pub mb_per_s: u64,
    pub reseed_ns: u64,
}

/// `None` sem relógio. Usa um gerador à parte, não o global.
pub fn benchmark() -> Option<Benchmark> {
    let mut rng = Rng::new(&[1]);
    let mut block = [0_u8; BENCH_BLOCK];
    let start = platform::now_ns()?;
    for _ in 0..BENCH_BLOCKS {
        rng.fill_bytes(&mut block);
    }
    let fill_ns = platform::now_ns()?.saturating_sub(start).max(1);
    let bytes = (BENCH_BLOCK * BENCH_BLOCKS) as u64;
    let start = platform::now_ns()?;
    replenish(&mut rng, &[platform::cycles()]);
    let reseed_ns = platform::now_ns()?.saturating_sub(start);
    wipe(&mut block);
    Some(Benchmark {
        mb_per_s: bytes * 1000 / fill_ns,
        reseed_ns,
    })
}
