//! Porta de entropia de hardware: o gerador da própria CPU, quando ela tem um.

pub trait HardwareEntropy {
    /// Enche `out` com bytes do gerador e diz de quais instruções eles vieram
    /// (`rdseed`, `rdrand`, `rdseed+rdrand`, `rndrrs`, `rndr` ou `rndrrs+rndr`).
    /// `None` se a instrução seguiu sinalizando falha depois das tentativas; o
    /// conteúdo de `out` é então indeterminado.
    fn fill(&self, out: &mut [u8]) -> Option<&'static str>;
}

/// Enche `out` com palavras de `next` (little-endian), repetindo até `attempts`
/// vezes a palavra em que `next` devolve `None`. `next` devolve a palavra e o bit da
/// instrução que a deu; o resultado é o OU dos bits usados, ou `None` se faltou
/// palavra. O resto do último pedaço é descartado.
pub fn fill_from_words(
    out: &mut [u8],
    attempts: u32,
    mut next: impl FnMut() -> Option<(u64, u8)>,
) -> Option<u8> {
    let mut used = 0;
    for chunk in out.chunks_mut(size_of::<u64>()) {
        let (word, source) = (0..attempts).find_map(|_| next())?;
        used |= source;
        chunk.copy_from_slice(&word.to_le_bytes()[..chunk.len()]);
    }
    Some(used)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_in_little_endian_and_truncates_the_tail() {
        let mut out = [0_u8; 11];
        let mut words = [0x0807_0605_0403_0201_u64, 0x1817_1615_1413_1211].into_iter();
        assert_eq!(
            fill_from_words(&mut out, 1, || words.next().map(|w| (w, 1))),
            Some(1)
        );
        assert_eq!(out, [1, 2, 3, 4, 5, 6, 7, 8, 0x11, 0x12, 0x13]);
    }

    #[test]
    fn retries_a_failing_word_up_to_the_limit() {
        let mut calls = 0;
        let mut out = [0_u8; 8];
        let used = fill_from_words(&mut out, 3, || {
            calls += 1;
            (calls == 3).then_some((7, 1))
        });
        assert!(used.is_some() && calls == 3);
        assert_eq!(out, 7_u64.to_le_bytes());
    }

    #[test]
    fn gives_up_after_the_attempts_run_out() {
        let mut calls = 0;
        let mut out = [0_u8; 16];
        let used = fill_from_words(&mut out, 4, || {
            calls += 1;
            None
        });
        assert_eq!(used, None);
        assert_eq!(calls, 4);
    }

    #[test]
    fn empty_output_needs_no_words() {
        assert_eq!(fill_from_words(&mut [], 1, || None), Some(0));
    }

    #[test]
    fn reports_every_source_that_gave_a_word() {
        let mut sources = [1_u8, 1, 2].into_iter();
        let mut out = [0_u8; 24];
        let used = fill_from_words(&mut out, 1, || sources.next().map(|s| (9, s)));
        assert_eq!(used, Some(0b11));
        let mut only_fallback = [2_u8, 2].into_iter();
        let used = fill_from_words(&mut out[..16], 1, || only_fallback.next().map(|s| (9, s)));
        assert_eq!(used, Some(0b10));
    }
}
