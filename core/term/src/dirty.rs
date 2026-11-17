//! Linhas que mudaram desde a última leitura, em bitmap de capacidade fixa.

const WORDS: usize = 4;

/// Máximo de linhas de uma grade: o bitmap de linhas sujas é fixo.
pub const MAX_ROWS: usize = WORDS * 64;

/// Conjunto de linhas sujas; itera em ordem crescente.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirtyRows {
    bits: [u64; WORDS],
}

impl DirtyRows {
    /// Nenhuma linha suja.
    pub const fn is_empty(&self) -> bool {
        let mut i = 0;
        while i < WORDS {
            if self.bits[i] != 0 {
                return false;
            }
            i += 1;
        }
        true
    }

    pub(crate) fn mark(&mut self, row: usize) {
        if row < MAX_ROWS {
            self.bits[row / 64] |= 1 << (row % 64);
        }
    }

    pub(crate) fn mark_range(&mut self, from: usize, to: usize) {
        for row in from..to.min(MAX_ROWS) {
            self.mark(row);
        }
    }

    pub(crate) fn drain(&mut self) -> DirtyRows {
        core::mem::take(self)
    }
}

#[allow(
    clippy::copy_iterator,
    reason = "o contrato pede DirtyRows Copy e Iterator"
)]
impl Iterator for DirtyRows {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        for (index, word) in self.bits.iter_mut().enumerate() {
            if *word != 0 {
                let bit = word.trailing_zeros() as usize;
                *word &= *word - 1;
                return Some(index * 64 + bit);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_set_is_empty() {
        assert!(DirtyRows::default().is_empty());
        assert_eq!(DirtyRows::default().next(), None);
    }

    #[test]
    fn marked_rows_come_out_in_ascending_order() {
        let mut dirty = DirtyRows::default();
        for row in [200, 3, 64, 63, 255, 0] {
            dirty.mark(row);
        }
        assert!(!dirty.is_empty());
        let rows: Vec<usize> = dirty.collect();
        assert_eq!(rows, [0, 3, 63, 64, 200, 255]);
    }

    #[test]
    fn marking_twice_yields_one_row() {
        let mut dirty = DirtyRows::default();
        dirty.mark(7);
        dirty.mark(7);
        assert_eq!(dirty.count(), 1);
    }

    #[test]
    fn rows_past_the_limit_are_ignored() {
        let mut dirty = DirtyRows::default();
        dirty.mark(MAX_ROWS);
        dirty.mark(usize::MAX);
        assert!(dirty.is_empty());
    }

    #[test]
    fn range_marks_half_open_and_clamps() {
        let mut dirty = DirtyRows::default();
        dirty.mark_range(2, 5);
        assert_eq!(dirty.collect::<Vec<_>>(), [2, 3, 4]);
        let mut dirty = DirtyRows::default();
        dirty.mark_range(250, 1000);
        assert_eq!(dirty.count(), 6);
    }

    #[test]
    fn drain_returns_the_set_and_clears_it() {
        let mut dirty = DirtyRows::default();
        dirty.mark(1);
        let taken = dirty.drain();
        assert_eq!(taken.count(), 1);
        assert!(dirty.is_empty());
    }

    #[test]
    fn copy_iterates_independently() {
        let mut dirty = DirtyRows::default();
        dirty.mark(9);
        let copy = dirty;
        assert_eq!(dirty.count(), 1);
        assert_eq!(copy.count(), 1);
    }
}
