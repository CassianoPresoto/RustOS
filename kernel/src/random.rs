//! Gerador aleatório do kernel: semente do hardware, ou jitter do contador quando a
//! CPU não tem gerador, e o estado global com reposição por volume entregue.

use alloc::boxed::Box;
use core::ptr;

use rustos_crypto::seed::{HARDWARE_SAMPLE_LEN, gather_jitter, hardware_output_plausible};
use rustos_crypto::{Rng, blake2b, wipe};
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
const BENCH_REQUESTS: u64 = 20_000;
/// Bytes da pilha, abaixo de um local do quadro de `stack_residue`, que o autoteste varre.
const SCAN_BYTES: usize = 2048;
/// Cópias que o autoteste planta para provar que a varredura enxerga.
pub const PLANTED: usize = 4;
/// As quatro primeiras palavras do estado do ChaCha20, que vêm antes da chave.
const CHACHA_CONSTANTS: [u8; 16] = *b"expand 32-byte k";

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

/// Vazão do ChaCha20 e custo de uma reposição, medidos no kernel com o relógio da fase 1.
#[derive(Clone, Copy, Debug)]
pub struct Benchmark {
    pub mb_per_s: u64,
    pub reseed_ns: u64,
    /// Décimos de ns por `Rng::next_u64`.
    pub u64_tenths_ns: u64,
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
    let start = platform::now_ns()?;
    for _ in 0..BENCH_REQUESTS {
        core::hint::black_box(rng.next_u64());
    }
    let u64_ns = platform::now_ns()?.saturating_sub(start);
    Some(Benchmark {
        mb_per_s: bytes * 1000 / fill_ns,
        reseed_ns,
        u64_tenths_ns: u64_ns * 10 / BENCH_REQUESTS,
    })
}

/// Cópias de material de chave achadas na pilha depois de semear e de um pedido.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Residue {
    /// Estados do ChaCha20 (constantes seguidas da chave).
    pub states: usize,
    /// Cópias dos 16 primeiros bytes da chave que o gerador tinha antes do pedido.
    pub keys: usize,
}

/// Enche a pilha abaixo do chamador com um padrão para o resíduo se distinguir.
#[inline(never)]
fn poison_stack() {
    let mut pad = [0x5A_u8; SCAN_BYTES + 512];
    core::hint::black_box(&mut pad);
}

/// Planta `PLANTED` estados falsos no quadro do primeiro callee, para o varredor provar
/// que enxerga a zona logo abaixo do chamador.
#[inline(never)]
fn plant_state_copies() {
    let copies = [CHACHA_CONSTANTS; PLANTED];
    core::hint::black_box(&copies);
}

/// Inline de propósito: um quadro próprio cairia sobre os primeiros bytes abaixo do `sp`,
/// onde fica o quadro do primeiro callee, e o resíduo ali não seria visto.
#[inline(always)]
#[allow(
    clippy::inline_always,
    reason = "sem quadro próprio a varredura não pisa no resíduo"
)]
fn count_windows(window: &[u8; 16], low: usize, high: usize) -> usize {
    let mut found = 0;
    for start in low..high.saturating_sub(window.len() - 1) {
        let matches = window.iter().enumerate().all(|(i, &byte)| {
            // SAFETY: `[low, high)` está na pilha própria do kernel, mapeada, abaixo do quadro.
            byte == unsafe { ptr::with_exposed_provenance::<u8>(start + i).read_volatile() }
        });
        found += usize::from(matches);
    }
    found
}

/// Semeia um gerador com a chave conhecida de `Rng::new(&[])`, atende um pedido e varre a
/// pilha abaixo daqui atrás de cópias do estado do ChaCha20 e da chave antiga (SDD 09).
/// Com `plant`, em vez do pedido planta `PLANTED` estados: a contagem tem de ser a deles.
#[inline(never)]
pub fn stack_residue(plant: bool) -> Residue {
    let mut known = Box::new([0_u8; 32]);
    let zero_key = [0_u8; 32];
    if blake2b::hash(&mut *known, &zero_key, &[]).is_err() {
        return Residue::default();
    }
    let needle: Box<[u8; 16]> = Box::new(known[..16].try_into().unwrap_or([0; 16]));
    let mut slot: Box<Option<Rng>> = Box::new(None);
    poison_stack();
    let mut out = Box::new([0_u8; 64]);
    if plant {
        plant_state_copies();
    } else {
        start(&mut slot).fill_bytes(&mut *out);
    }
    let probe = 0_u8;
    let top = ptr::from_ref(&probe).expose_provenance();
    let (low, high) = (top - SCAN_BYTES, top);
    Residue {
        states: count_windows(&CHACHA_CONSTANTS, low, high),
        keys: count_windows(&needle, low, high),
    }
}
