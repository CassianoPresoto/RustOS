//! Marca d'água de pilha: onde, a partir do fundo, o padrão de pintura deixa de aparecer.

/// Palavras por grupo: um OR de XORs por grupo troca um desvio por palavra.
const GROUP: usize = 8;

/// Índice da primeira palavra, contando do fundo (índice 0), que difere de `pattern`; `words`
/// se todas são iguais a ele. Exato: olha toda palavra até achar a primeira suja.
pub fn first_unpainted(words: usize, pattern: u64, read: impl Fn(usize) -> u64) -> usize {
    let dirty = |index: &usize| read(*index) != pattern;
    let mut base = 0;
    while base + GROUP <= words {
        let differs = (base..base + GROUP).fold(0, |acc, index| acc | (read(index) ^ pattern));
        if differs != 0 {
            return (base..base + GROUP).find(dirty).unwrap_or(base);
        }
        base += GROUP;
    }
    (base..words).find(dirty).unwrap_or(words)
}

/// Varre para baixo a partir da palavra `from` (exclusiva) e devolve o índice da palavra suja
/// mais funda que ainda tem sujeira acima de um trecho de `run` palavras limpas seguidas;
/// `from` se não há nenhuma. Quem chama repinta `[resultado, from)`: é a limpeza da pilha do
/// kernel na volta ao usuário (SDD 17), que custa a profundidade que a chamada usou.
pub fn lowest_dirty_below(
    from: usize,
    pattern: u64,
    run: usize,
    read: impl Fn(usize) -> u64,
) -> usize {
    let (mut index, mut lowest, mut clean) = (from, from, 0);
    while index > 0 && clean < run {
        index -= 1;
        if read(index) == pattern {
            clean += 1;
        } else {
            lowest = index;
            clean = 0;
        }
    }
    lowest
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATTERN: u64 = 0x5253_5441_434b_2121;

    fn painted(words: usize, dirty: &[usize]) -> Vec<u64> {
        let mut memory = vec![PATTERN; words];
        for &index in dirty {
            memory[index] = 1;
        }
        memory
    }

    fn first(memory: &[u64]) -> usize {
        first_unpainted(memory.len(), PATTERN, |index| memory[index])
    }

    #[test]
    fn an_empty_or_untouched_stack_is_all_painted() {
        assert_eq!(first(&[]), 0);
        assert_eq!(first(&painted(2048, &[])), 2048);
        assert_eq!(first(&painted(13, &[])), 13);
    }

    #[test]
    fn a_full_stack_is_dirty_from_the_bottom() {
        assert_eq!(first(&painted(2048, &[0])), 0);
        assert_eq!(first(&vec![0; 64]), 0);
    }

    #[test]
    fn the_first_dirty_word_is_found_in_any_position_of_a_group() {
        for at in 0..40 {
            assert_eq!(first(&painted(40, &[at, 39])), at);
        }
    }

    #[test]
    fn a_tail_shorter_than_a_group_is_scanned_too() {
        assert_eq!(first(&painted(13, &[12])), 12);
        assert_eq!(first(&painted(13, &[9, 11])), 9);
    }

    #[test]
    fn holes_above_the_deepest_write_do_not_move_the_result() {
        let mut memory = painted(512, &[]);
        for index in (200..512).step_by(3) {
            memory[index] = 7;
        }
        assert_eq!(first(&memory), 200);
    }

    #[test]
    fn it_agrees_with_a_plain_scan_on_random_holes() {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        for words in [1, 7, 8, 9, 63, 64, 65, 1000] {
            for _ in 0..200 {
                let mut memory = vec![PATTERN; words];
                for slot in &mut memory {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    if seed % 10 < 3 {
                        *slot = seed | 1;
                    }
                }
                let plain = memory.iter().position(|&word| word != PATTERN);
                assert_eq!(first(&memory), plain.unwrap_or(words));
            }
        }
    }

    fn lowest(memory: &[u64], from: usize, run: usize) -> usize {
        lowest_dirty_below(from, PATTERN, run, |index| memory[index])
    }

    #[test]
    fn a_clean_stack_has_nothing_to_scrub() {
        let memory = painted(64, &[]);
        assert_eq!(lowest(&memory, 64, 16), 64);
        assert_eq!(lowest(&memory, 0, 16), 0);
    }

    #[test]
    fn the_scan_stops_after_a_run_of_clean_words() {
        let memory = painted(256, &[250, 240, 100]);
        assert_eq!(lowest(&memory, 256, 16), 240);
        assert_eq!(lowest(&memory, 256, 200), 100);
    }

    #[test]
    fn a_dirty_word_inside_the_run_restarts_the_count() {
        let memory = painted(256, &[254, 249, 244, 239]);
        assert_eq!(lowest(&memory, 256, 6), 239);
        assert_eq!(lowest(&memory, 256, 4), 254);
    }

    #[test]
    fn a_fully_dirty_stack_is_scanned_to_the_bottom() {
        let memory = vec![0; 128];
        assert_eq!(lowest(&memory, 128, 16), 0);
    }

    #[test]
    fn only_what_the_run_reaches_below_the_start_word_is_scrubbed() {
        let memory = painted(64, &[40, 3]);
        assert_eq!(lowest(&memory, 32, 8), 32);
        assert_eq!(lowest(&memory, 32, 40), 3);
    }
}
