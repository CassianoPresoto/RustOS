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
}
