//! Semente de reserva por jitter do contador e conferência da semente de hardware.
//! Puro: o contador (`rdtsc`, `CNTVCT`) vem de fora, como função.

use core::hint::black_box;

use crate::blake2b::Blake2b;

/// Bytes da saída de `gather_jitter`.
pub const JITTER_LEN: usize = 32;
/// Bytes que `hardware_output_plausible` confere: 8 palavras de 64 bits.
pub const HARDWARE_SAMPLE_LEN: usize = 64;

/// Passos do trabalho entre duas leituras; o tempo dele varia com cache e pipeline.
const WORK_STEPS: usize = 16;
/// Palavras consecutivas que têm de diferir: 5 de 7, como o `x86_init_rdrand` do Linux.
const MIN_CHANGES: usize = 5;

/// Lê o contador `rounds` vezes, com um trabalho curto entre as leituras, e devolve
/// o BLAKE2b-256 das diferenças e dos valores lidos. Fonte fraca: só o ruído de tempo.
pub fn gather_jitter(mut counter: impl FnMut() -> u64, rounds: usize) -> [u8; JITTER_LEN] {
    let mut hash = Blake2b::init(JITTER_LEN, &[]);
    let mut table = [0_u64; WORK_STEPS];
    let mut last = counter();
    for round in 0..rounds {
        let mut cursor = round;
        for step in 0..WORK_STEPS {
            cursor = usize::try_from(table[cursor % WORK_STEPS] >> 7).unwrap_or(0) ^ cursor;
            table[step] = table[step].rotate_left(5) ^ (cursor as u64).wrapping_add(step as u64);
            black_box(&mut table);
        }
        let now = counter();
        hash.update(&now.wrapping_sub(last).to_le_bytes());
        hash.update(&now.to_le_bytes());
        last = now;
    }
    for word in &table {
        hash.update(&word.to_le_bytes());
    }
    let mut out = [0_u8; JITTER_LEN];
    hash.finish_into(&mut out);
    out
}

/// Recusa saída de hardware travada: palavras iguais em sequência, como já houve
/// em defeito de CPU. Não prova qualidade; só pega o instrumento quebrado.
pub fn hardware_output_plausible(sample: &[u8; HARDWARE_SAMPLE_LEN]) -> bool {
    let words = sample.as_chunks::<8>().0;
    let changes = words.windows(2).filter(|pair| pair[0] != pair[1]).count();
    changes >= MIN_CHANGES
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticking(step: u64) -> impl FnMut() -> u64 {
        let mut now = 0;
        move || {
            now += step;
            now
        }
    }

    #[test]
    fn jitter_is_deterministic_for_the_same_readings() {
        assert_eq!(gather_jitter(ticking(7), 64), gather_jitter(ticking(7), 64));
    }

    #[test]
    fn jitter_depends_on_the_readings_and_on_the_rounds() {
        let base = gather_jitter(ticking(7), 64);
        assert_ne!(base, gather_jitter(ticking(8), 64));
        assert_ne!(base, gather_jitter(ticking(7), 65));
        assert_ne!(base, [0; JITTER_LEN]);
    }

    #[test]
    fn jitter_reads_the_counter_once_before_and_once_per_round() {
        let mut reads = 0;
        gather_jitter(
            || {
                reads += 1;
                reads
            },
            10,
        );
        assert_eq!(reads, 11);
    }

    #[test]
    fn jitter_with_zero_rounds_still_returns() {
        assert_ne!(gather_jitter(ticking(1), 0), [0; JITTER_LEN]);
    }

    fn sample(words: [u64; 8]) -> [u8; HARDWARE_SAMPLE_LEN] {
        let mut out = [0; HARDWARE_SAMPLE_LEN];
        for (chunk, word) in out.chunks_mut(8).zip(words) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        out
    }

    #[test]
    fn stuck_hardware_output_is_rejected() {
        assert!(!hardware_output_plausible(&sample([0; 8])));
        assert!(!hardware_output_plausible(&sample([u64::MAX; 8])));
        assert!(!hardware_output_plausible(&sample([
            1, 1, 1, 1, 1, 1, 1, 2
        ])));
    }

    #[test]
    fn varying_hardware_output_is_accepted() {
        assert!(hardware_output_plausible(&sample([1, 2, 3, 4, 5, 6, 7, 8])));
        assert!(hardware_output_plausible(&sample([1, 2, 1, 2, 1, 2, 1, 2])));
        assert!(!hardware_output_plausible(&sample([
            1, 1, 2, 2, 3, 3, 4, 4
        ])));
    }
}
