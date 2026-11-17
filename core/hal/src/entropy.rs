//! Porta de entropia de hardware: o gerador da própria CPU, quando ela tem um.

pub trait HardwareEntropy {
    /// A instrução que atende o pedido: `rdseed`, `rdrand`, `rndrrs` ou `rndr`.
    fn name(&self) -> &'static str;

    /// Enche `out` com bytes do gerador. `false` se a instrução seguiu sinalizando
    /// falha depois das tentativas; o conteúdo de `out` é então indeterminado.
    fn fill(&self, out: &mut [u8]) -> bool;
}

/// Enche `out` com palavras de `next` (little-endian), repetindo até `attempts`
/// vezes a palavra em que `next` devolve `None`. O resto do último pedaço é descartado.
pub fn fill_from_words(
    out: &mut [u8],
    attempts: u32,
    mut next: impl FnMut() -> Option<u64>,
) -> bool {
    for chunk in out.chunks_mut(size_of::<u64>()) {
        let Some(word) = (0..attempts).find_map(|_| next()) else {
            return false;
        };
        chunk.copy_from_slice(&word.to_le_bytes()[..chunk.len()]);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_in_little_endian_and_truncates_the_tail() {
        let mut out = [0_u8; 11];
        let mut words = [0x0807_0605_0403_0201_u64, 0x1817_1615_1413_1211].into_iter();
        assert!(fill_from_words(&mut out, 1, || words.next()));
        assert_eq!(out, [1, 2, 3, 4, 5, 6, 7, 8, 0x11, 0x12, 0x13]);
    }

    #[test]
    fn retries_a_failing_word_up_to_the_limit() {
        let mut calls = 0;
        let mut out = [0_u8; 8];
        let ok = fill_from_words(&mut out, 3, || {
            calls += 1;
            (calls == 3).then_some(7)
        });
        assert!(ok && calls == 3);
        assert_eq!(out, 7_u64.to_le_bytes());
    }

    #[test]
    fn gives_up_after_the_attempts_run_out() {
        let mut calls = 0;
        let mut out = [0_u8; 16];
        let ok = fill_from_words(&mut out, 4, || {
            calls += 1;
            None
        });
        assert!(!ok);
        assert_eq!(calls, 4);
    }

    #[test]
    fn empty_output_needs_no_words() {
        assert!(fill_from_words(&mut [], 1, || None));
    }
}
