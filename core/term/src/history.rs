//! Anel de linhas que saíram do topo da tela, sobre memória que vem de fora.

use crate::cell::Cell;

/// Linhas antigas da tela principal; a mais antiga sai quando o anel enche.
#[derive(Debug)]
pub(crate) struct History<'a> {
    cells: &'a mut [Cell],
    cols: usize,
    capacity: usize,
    start: usize,
    len: usize,
}

impl<'a> History<'a> {
    pub(crate) fn new(cells: &'a mut [Cell], cols: usize) -> Self {
        let capacity = cells.len() / cols;
        History {
            cells,
            cols,
            capacity,
            start: 0,
            len: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    /// Guarda `row` (`cols` células) como a mais nova; sem capacidade, não faz nada.
    pub(crate) fn push(&mut self, row: &[Cell]) {
        if self.capacity == 0 {
            return;
        }
        let slot = if self.len < self.capacity {
            self.len += 1;
            (self.start + self.len - 1) % self.capacity
        } else {
            let oldest = self.start;
            self.start = (self.start + 1) % self.capacity;
            oldest
        };
        self.cells[slot * self.cols..(slot + 1) * self.cols].copy_from_slice(row);
    }

    /// A linha `index`, com 0 a mais antiga.
    pub(crate) fn get(&self, index: usize) -> Option<&[Cell]> {
        if index >= self.len {
            return None;
        }
        let slot = (self.start + index) % self.capacity;
        Some(&self.cells[slot * self.cols..(slot + 1) * self.cols])
    }

    pub(crate) fn clear(&mut self) {
        self.start = 0;
        self.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ch: char, cols: usize) -> Vec<Cell> {
        vec![Cell { ch, ..Cell::BLANK }; cols]
    }

    fn text(history: &History<'_>) -> String {
        (0..history.len())
            .map(|i| history.get(i).unwrap()[0].ch)
            .collect()
    }

    #[test]
    fn without_memory_nothing_is_kept() {
        let mut history = History::new(&mut [], 4);
        history.push(&row('a', 4));
        assert_eq!((history.len(), history.capacity()), (0, 0));
        assert!(history.get(0).is_none());
    }

    #[test]
    fn capacity_counts_whole_lines_only() {
        let mut cells = vec![Cell::BLANK; 11];
        assert_eq!(History::new(&mut cells, 4).capacity(), 2);
    }

    #[test]
    fn lines_come_out_oldest_first() {
        let mut cells = vec![Cell::BLANK; 12];
        let mut history = History::new(&mut cells, 4);
        history.push(&row('a', 4));
        history.push(&row('b', 4));
        assert_eq!(text(&history), "ab");
        assert_eq!(history.get(1).unwrap(), row('b', 4).as_slice());
        assert!(history.get(2).is_none());
    }

    #[test]
    fn a_full_ring_drops_the_oldest_line() {
        let mut cells = vec![Cell::BLANK; 12];
        let mut history = History::new(&mut cells, 4);
        for ch in "abcde".chars() {
            history.push(&row(ch, 4));
        }
        assert_eq!(text(&history), "cde");
        history.push(&row('f', 4));
        assert_eq!(text(&history), "def");
    }

    #[test]
    fn clear_empties_and_reuses_the_ring() {
        let mut cells = vec![Cell::BLANK; 8];
        let mut history = History::new(&mut cells, 4);
        for ch in "abc".chars() {
            history.push(&row(ch, 4));
        }
        history.clear();
        assert_eq!(history.len(), 0);
        history.push(&row('z', 4));
        assert_eq!(text(&history), "z");
    }
}
