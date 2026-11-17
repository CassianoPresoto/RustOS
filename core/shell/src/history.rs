//! Histórico: anel de linhas de capacidade fixa, da mais nova para a mais velha.

/// Leitura do histórico, usada pelo comando `history`.
pub(crate) trait Entries {
    fn count(&self) -> usize;
    /// `n`-ésima linha a partir da mais antiga, com o número que o `history` mostra.
    fn nth_oldest(&self, n: usize) -> Option<(u32, &str)>;
}

/// Anel de `N` linhas de até `LINE` bytes. Cheio, a mais antiga sai.
#[derive(Clone, Copy, Debug)]
pub(crate) struct History<const LINE: usize, const N: usize> {
    lines: [[u8; LINE]; N],
    lens: [u16; N],
    head: usize,
    count: usize,
    total: u32,
}

impl<const LINE: usize, const N: usize> History<LINE, N> {
    pub(crate) const fn new() -> Self {
        const {
            assert!(
                LINE <= u16::MAX as usize,
                "linha maior que o campo de tamanho"
            );
        };
        Self {
            lines: [[0; LINE]; N],
            lens: [0; N],
            head: 0,
            count: 0,
            total: 0,
        }
    }

    /// Linha de idade `age`: 0 é a mais recente.
    pub(crate) fn get(&self, age: usize) -> Option<&str> {
        if age >= self.count {
            return None;
        }
        let slot = (self.head + N - 1 - age) % N;
        let len = usize::from(self.lens[slot]);
        core::str::from_utf8(&self.lines[slot][..len]).ok()
    }

    /// Guarda a linha. `false` se for vazia, só espaços, igual à mais recente ou grande demais.
    pub(crate) fn push(&mut self, line: &str) -> bool {
        let fits = line.len() <= LINE && N > 0;
        if !fits || line.trim().is_empty() || self.get(0) == Some(line) {
            return false;
        }
        self.lines[self.head][..line.len()].copy_from_slice(line.as_bytes());
        self.lens[self.head] = u16::try_from(line.len()).unwrap_or(u16::MAX);
        self.head = (self.head + 1) % N;
        self.count = (self.count + 1).min(N);
        self.total = self.total.saturating_add(1);
        true
    }
}

impl<const LINE: usize, const N: usize> Entries for History<LINE, N> {
    fn count(&self) -> usize {
        self.count
    }

    fn nth_oldest(&self, n: usize) -> Option<(u32, &str)> {
        let age = self.count.checked_sub(1 + n)?;
        let skipped = self.total - u32::try_from(self.count).unwrap_or(u32::MAX);
        let number = skipped.saturating_add(u32::try_from(n + 1).unwrap_or(u32::MAX));
        Some((number, self.get(age)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_history_is_empty() {
        let history = History::<8, 3>::new();
        assert_eq!(history.count(), 0);
        assert_eq!(history.get(0), None);
        assert_eq!(history.nth_oldest(0), None);
    }

    #[test]
    fn get_returns_newest_first() {
        let mut history = History::<8, 3>::new();
        assert!(history.push("one"));
        assert!(history.push("two"));
        assert_eq!(history.get(0), Some("two"));
        assert_eq!(history.get(1), Some("one"));
        assert_eq!(history.get(2), None);
    }

    #[test]
    fn full_ring_drops_the_oldest() {
        let mut history = History::<8, 3>::new();
        for line in ["a", "b", "c", "d"] {
            history.push(line);
        }
        assert_eq!(history.count(), 3);
        assert_eq!(history.get(0), Some("d"));
        assert_eq!(history.get(2), Some("b"));
    }

    #[test]
    fn blank_and_repeated_lines_are_skipped() {
        let mut history = History::<8, 3>::new();
        assert!(!history.push(""));
        assert!(!history.push("  \t"));
        assert!(history.push("ls"));
        assert!(!history.push("ls"));
        assert!(history.push("pwd"));
        assert!(history.push("ls"));
        assert_eq!(history.count(), 3);
    }

    #[test]
    fn line_longer_than_capacity_is_refused() {
        let mut history = History::<4, 2>::new();
        assert!(!history.push("abcde"));
        assert!(history.push("abcd"));
    }

    #[test]
    fn zero_sized_history_ignores_everything() {
        let mut history = History::<8, 0>::new();
        assert!(!history.push("ls"));
        assert_eq!(history.count(), 0);
        assert_eq!(history.get(0), None);
    }

    #[test]
    fn numbering_keeps_counting_after_the_ring_wraps() {
        let mut history = History::<8, 2>::new();
        for line in ["a", "b", "c", "d", "e"] {
            history.push(line);
        }
        assert_eq!(history.count(), 2);
        assert_eq!(history.nth_oldest(0), Some((4, "d")));
        assert_eq!(history.nth_oldest(1), Some((5, "e")));
        assert_eq!(history.nth_oldest(2), None);
    }

    #[test]
    fn skipped_lines_do_not_consume_numbers() {
        let mut history = History::<8, 4>::new();
        history.push("a");
        history.push("a");
        history.push(" ");
        history.push("b");
        assert_eq!(history.nth_oldest(1), Some((2, "b")));
    }
}
