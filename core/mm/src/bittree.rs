//! Bitmap hierárquico: um bit por bloco e, acima, um bit por palavra não vazia.
//! Achar o menor bit ligado custa um passo por nível, não uma varredura.

const WORD_BITS: usize = u64::BITS as usize;
const LEVEL_SHIFT: usize = WORD_BITS.trailing_zeros() as usize;
pub const MAX_LEVELS: usize = 5;

/// Níveis de uma árvore sobre `bits` folhas; o último tem uma palavra só.
pub const fn depth_for(bits: usize) -> usize {
    let (mut depth, mut count) = (0, if bits == 0 { 1 } else { bits });
    loop {
        depth += 1;
        count = count.div_ceil(WORD_BITS);
        if count == 1 {
            return depth;
        }
    }
}

/// Palavras de `u64` que a árvore ocupa, somados todos os níveis.
pub const fn words_for(bits: usize) -> usize {
    let (mut total, mut count) = (0, if bits == 0 { 1 } else { bits });
    loop {
        count = count.div_ceil(WORD_BITS);
        total += count;
        if count == 1 {
            return total;
        }
    }
}

/// Visão de uma árvore dentro de uma fatia de palavras; quem guarda a fatia é o dono.
#[derive(Clone, Copy, Debug)]
pub struct BitTree {
    offsets: [usize; MAX_LEVELS],
    depth: usize,
    bits: usize,
}

impl BitTree {
    pub const EMPTY: Self = Self {
        offsets: [0; MAX_LEVELS],
        depth: 1,
        bits: 0,
    };

    /// `None` se `bits` pede mais níveis do que `MAX_LEVELS`.
    pub fn at(start: usize, bits: usize) -> Option<Self> {
        let depth = depth_for(bits);
        if depth > MAX_LEVELS {
            return None;
        }
        let mut offsets = [0; MAX_LEVELS];
        let (mut offset, mut count) = (start, bits.max(1));
        for slot in &mut offsets[..depth] {
            *slot = offset;
            count = count.div_ceil(WORD_BITS);
            offset += count;
        }
        Some(Self {
            offsets,
            depth,
            bits,
        })
    }

    pub fn test(&self, store: &[u64], index: usize) -> bool {
        debug_assert!(index < self.bits);
        store[self.offsets[0] + (index >> LEVEL_SHIFT)] & (1 << (index % WORD_BITS)) != 0
    }

    pub fn set(&self, store: &mut [u64], index: usize) {
        debug_assert!(index < self.bits);
        for level in 0..self.depth {
            let position = index >> (LEVEL_SHIFT * level);
            let word = &mut store[self.offsets[level] + (position >> LEVEL_SHIFT)];
            let was = *word;
            *word |= 1 << (position % WORD_BITS);
            if was != 0 {
                return;
            }
        }
    }

    pub fn clear(&self, store: &mut [u64], index: usize) {
        debug_assert!(index < self.bits);
        for level in 0..self.depth {
            let position = index >> (LEVEL_SHIFT * level);
            let word = &mut store[self.offsets[level] + (position >> LEVEL_SHIFT)];
            *word &= !(1 << (position % WORD_BITS));
            if *word != 0 {
                return;
            }
        }
    }

    pub fn is_empty(&self, store: &[u64]) -> bool {
        store[self.offsets[self.depth - 1]] == 0
    }

    /// O menor índice ligado.
    pub fn first_set(&self, store: &[u64]) -> Option<usize> {
        let top = store[self.offsets[self.depth - 1]];
        if top == 0 {
            return None;
        }
        let mut index = top.trailing_zeros() as usize;
        for level in (0..self.depth - 1).rev() {
            let word = store[self.offsets[level] + index];
            index = index * WORD_BITS + word.trailing_zeros() as usize;
        }
        Some(index)
    }

    /// Algum bit de `[low, high)` ligado, olhando só as folhas.
    pub fn any_in(&self, store: &[u64], low: usize, high: usize) -> bool {
        debug_assert!(high <= self.bits);
        let mut index = low;
        while index < high {
            let word = store[self.offsets[0] + (index >> LEVEL_SHIFT)];
            let from = index % WORD_BITS;
            let to = (high - (index - from)).min(WORD_BITS);
            let mask = if to - from == WORD_BITS {
                u64::MAX
            } else {
                ((1u64 << (to - from)) - 1) << from
            };
            if word & mask != 0 {
                return true;
            }
            index += to - from;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(bits: usize) -> (BitTree, Vec<u64>) {
        (BitTree::at(0, bits).unwrap(), vec![0; words_for(bits)])
    }

    #[test]
    fn counts_levels_and_words_for_each_size() {
        assert_eq!((depth_for(0), words_for(0)), (1, 1));
        assert_eq!((depth_for(64), words_for(64)), (1, 1));
        assert_eq!((depth_for(65), words_for(65)), (2, 3));
        assert_eq!((depth_for(4096), words_for(4096)), (2, 65));
        assert_eq!((depth_for(4097), words_for(4097)), (3, 68));
    }

    #[test]
    fn finds_the_lowest_set_bit_across_levels() {
        let (tree, mut store) = tree(100_000);
        assert_eq!(tree.first_set(&store), None);
        for index in [99_999, 70_000, 4_095, 64] {
            tree.set(&mut store, index);
        }
        let mut seen = Vec::new();
        while let Some(index) = tree.first_set(&store) {
            seen.push(index);
            tree.clear(&mut store, index);
        }
        assert_eq!(seen, [64, 4_095, 70_000, 99_999]);
        assert!(store.iter().all(|&word| word == 0));
    }

    #[test]
    fn clearing_one_bit_keeps_the_summary_of_its_neighbours() {
        let (tree, mut store) = tree(10_000);
        tree.set(&mut store, 10);
        tree.set(&mut store, 11);
        tree.clear(&mut store, 10);
        assert!(tree.test(&store, 11));
        assert_eq!(tree.first_set(&store), Some(11));
    }

    #[test]
    fn any_in_respects_word_edges() {
        let (tree, mut store) = tree(300);
        tree.set(&mut store, 64);
        assert!(!tree.any_in(&store, 0, 64));
        assert!(tree.any_in(&store, 0, 65));
        assert!(tree.any_in(&store, 64, 65));
        assert!(!tree.any_in(&store, 65, 300));
        assert!(!tree.any_in(&store, 10, 10));
    }

    #[test]
    fn refuses_more_levels_than_the_limit() {
        assert!(BitTree::at(0, 1 << 30).is_some());
        assert!(BitTree::at(0, 1 << 31).is_none());
    }
}
