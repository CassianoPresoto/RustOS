//! Paradas de tabulação, em bitmap de capacidade fixa.

const WORDS: usize = 8;

/// Máximo de colunas de uma grade: o bitmap de paradas é fixo.
pub const MAX_COLS: usize = WORDS * 64;

/// Conjunto de colunas com parada de tabulação; o padrão é uma a cada 8 colunas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TabStops {
    bits: [u64; WORDS],
}

impl TabStops {
    pub(crate) const fn every_eight() -> Self {
        TabStops {
            bits: [0x0101_0101_0101_0101; WORDS],
        }
    }

    pub(crate) fn set(&mut self, col: usize) {
        if col < MAX_COLS {
            self.bits[col / 64] |= 1 << (col % 64);
        }
    }

    pub(crate) fn clear(&mut self, col: usize) {
        if col < MAX_COLS {
            self.bits[col / 64] &= !(1 << (col % 64));
        }
    }

    pub(crate) fn clear_all(&mut self) {
        self.bits = [0; WORDS];
    }

    fn is_set(&self, col: usize) -> bool {
        self.bits[col / 64] & (1 << (col % 64)) != 0
    }

    /// Próxima parada depois de `col`, limitada a `last`; uma parada em `col` não conta.
    pub(crate) fn next(&self, col: usize, last: usize) -> usize {
        let last = last.min(MAX_COLS - 1);
        (col + 1..=last).find(|&c| self.is_set(c)).unwrap_or(last)
    }

    /// Parada anterior a `col`, ou a coluna 0; uma parada em `col` não conta.
    pub(crate) fn prev(&self, col: usize) -> usize {
        (0..col.min(MAX_COLS))
            .rev()
            .find(|&c| self.is_set(c))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_stops_sit_every_eight_columns() {
        let tabs = TabStops::every_eight();
        assert_eq!(tabs.next(0, 79), 8);
        assert_eq!(tabs.next(7, 79), 8);
        assert_eq!(tabs.next(8, 79), 16);
        assert_eq!(tabs.next(500, 600), 504);
        assert_eq!(tabs.prev(9), 8);
        assert_eq!(tabs.prev(8), 0);
        assert_eq!(tabs.prev(0), 0);
    }

    #[test]
    fn next_stops_at_the_last_column_when_there_is_no_stop() {
        let tabs = TabStops::every_eight();
        assert_eq!(tabs.next(76, 79), 79);
        assert_eq!(tabs.next(79, 79), 79);
        assert_eq!(tabs.next(2, 5), 5);
    }

    #[test]
    fn set_and_clear_touch_one_column() {
        let mut tabs = TabStops::every_eight();
        tabs.set(3);
        assert_eq!(tabs.next(0, 79), 3);
        tabs.clear(3);
        tabs.clear(8);
        assert_eq!(tabs.next(0, 79), 16);
        tabs.clear(1);
    }

    #[test]
    fn clear_all_leaves_only_the_edges() {
        let mut tabs = TabStops::every_eight();
        tabs.clear_all();
        assert_eq!(tabs.next(0, 79), 79);
        assert_eq!(tabs.prev(40), 0);
    }

    #[test]
    fn columns_past_the_capacity_are_ignored() {
        let mut tabs = TabStops::every_eight();
        tabs.set(MAX_COLS);
        tabs.clear(usize::MAX);
        assert_eq!(tabs.next(MAX_COLS - 2, usize::MAX), MAX_COLS - 1);
        assert_eq!(tabs.prev(usize::MAX), MAX_COLS - 8);
    }
}
