//! Linha em edição: bytes UTF-8 de capacidade fixa e um cursor sempre em fronteira de caractere.

/// Linha de até `N` bytes. Os bytes são sempre UTF-8 válido e o cursor nunca cai no meio
/// de um caractere.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LineBuffer<const N: usize> {
    bytes: [u8; N],
    len: usize,
    cursor: usize,
}

fn is_continuation(byte: u8) -> bool {
    byte & 0xc0 == 0x80
}

fn is_blank(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

impl<const N: usize> LineBuffer<N> {
    pub(crate) const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
            cursor: 0,
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) const fn free(&self) -> usize {
        N - self.len
    }

    /// Posição do cursor em bytes.
    pub(crate) const fn cursor(&self) -> usize {
        self.cursor
    }

    pub(crate) fn before_cursor(&self) -> &str {
        self.as_str().get(..self.cursor).unwrap_or("")
    }

    pub(crate) fn after_cursor(&self) -> &str {
        self.as_str().get(self.cursor..).unwrap_or("")
    }

    /// Caracteres na linha; cada um ocupa uma célula na tela.
    pub(crate) fn char_len(&self) -> usize {
        self.as_str().chars().count()
    }

    /// Caracteres antes do cursor.
    pub(crate) fn char_cursor(&self) -> usize {
        self.before_cursor().chars().count()
    }

    pub(crate) fn clear(&mut self) {
        self.len = 0;
        self.cursor = 0;
    }

    /// Troca o conteúdo por `text`, cortado na fronteira de caractere que cabe, cursor no fim.
    pub(crate) fn set(&mut self, text: &str) {
        let mut end = text.len().min(N);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.bytes[..end].copy_from_slice(&text.as_bytes()[..end]);
        self.len = end;
        self.cursor = end;
    }

    /// Insere no cursor; `false` e nada muda se não couber.
    pub(crate) fn insert(&mut self, c: char) -> bool {
        let mut encoded = [0; 4];
        self.insert_str(c.encode_utf8(&mut encoded))
    }

    /// Insere o texto inteiro no cursor; `false` e nada muda se não couber.
    pub(crate) fn insert_str(&mut self, text: &str) -> bool {
        let size = text.len();
        if size > self.free() {
            return false;
        }
        self.bytes
            .copy_within(self.cursor..self.len, self.cursor + size);
        self.bytes[self.cursor..self.cursor + size].copy_from_slice(text.as_bytes());
        self.len += size;
        self.cursor += size;
        true
    }

    fn previous_boundary(&self) -> usize {
        let mut at = self.cursor;
        while at > 0 {
            at -= 1;
            if !is_continuation(self.bytes[at]) {
                break;
            }
        }
        at
    }

    fn next_boundary(&self) -> usize {
        let mut at = self.cursor;
        while at < self.len {
            at += 1;
            if at == self.len || !is_continuation(self.bytes[at]) {
                break;
            }
        }
        at
    }

    fn remove(&mut self, from: usize, to: usize) -> bool {
        if from == to {
            return false;
        }
        self.bytes.copy_within(to..self.len, from);
        self.len -= to - from;
        self.cursor = from;
        true
    }

    /// Apaga o caractere antes do cursor.
    pub(crate) fn backspace(&mut self) -> bool {
        self.remove(self.previous_boundary(), self.cursor)
    }

    /// Apaga o caractere sob o cursor.
    pub(crate) fn delete(&mut self) -> bool {
        let cursor = self.cursor;
        self.remove(cursor, self.next_boundary())
    }

    /// Apaga do cursor ao fim.
    pub(crate) fn kill_to_end(&mut self) -> bool {
        let cursor = self.cursor;
        self.remove(cursor, self.len)
    }

    /// Apaga do início ao cursor.
    pub(crate) fn kill_to_start(&mut self) -> bool {
        self.remove(0, self.cursor)
    }

    /// Apaga a palavra antes do cursor e os espaços entre ela e o cursor.
    pub(crate) fn kill_word(&mut self) -> bool {
        let mut start = self.cursor;
        while start > 0 && is_blank(self.bytes[start - 1]) {
            start -= 1;
        }
        while start > 0 && !is_blank(self.bytes[start - 1]) {
            start -= 1;
        }
        self.remove(start, self.cursor)
    }

    /// Anda um caractere para a esquerda.
    pub(crate) fn left(&mut self) -> bool {
        let moved = self.cursor > 0;
        self.cursor = self.previous_boundary();
        moved
    }

    /// Anda um caractere para a direita.
    pub(crate) fn right(&mut self) -> bool {
        let moved = self.cursor < self.len;
        self.cursor = self.next_boundary();
        moved
    }

    pub(crate) fn home(&mut self) -> bool {
        let moved = self.cursor > 0;
        self.cursor = 0;
        moved
    }

    pub(crate) fn end(&mut self) -> bool {
        let moved = self.cursor < self.len;
        self.cursor = self.len;
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> LineBuffer<16> {
        let mut line = LineBuffer::new();
        line.set(text);
        line
    }

    #[test]
    fn new_line_is_empty_with_full_capacity() {
        let line = LineBuffer::<8>::new();
        assert!(line.is_empty());
        assert_eq!((line.len(), line.cursor(), line.free()), (0, 0, 8));
        assert_eq!(line.as_str(), "");
    }

    #[test]
    fn insert_goes_at_the_cursor() {
        let mut line = line("ac");
        line.left();
        assert!(line.insert('b'));
        assert_eq!(line.as_str(), "abc");
        assert_eq!(line.before_cursor(), "ab");
        assert_eq!(line.after_cursor(), "c");
    }

    #[test]
    fn insert_refuses_when_full_and_changes_nothing() {
        let mut line = LineBuffer::<3>::new();
        assert!(line.insert_str("abc"));
        assert!(!line.insert('d'));
        assert_eq!(line.as_str(), "abc");
        assert_eq!(line.cursor(), 3);
    }

    #[test]
    fn insert_refuses_a_multibyte_char_that_does_not_fit() {
        let mut line = LineBuffer::<3>::new();
        assert!(line.insert_str("ab"));
        assert!(!line.insert('é'));
        assert_eq!(line.as_str(), "ab");
    }

    #[test]
    fn insert_str_is_all_or_nothing() {
        let mut line = LineBuffer::<4>::new();
        assert!(!line.insert_str("abcde"));
        assert!(line.is_empty());
        assert!(line.insert_str("abcd"));
        assert_eq!(line.free(), 0);
    }

    #[test]
    fn backspace_removes_one_whole_multibyte_char() {
        let mut line = line("aé€");
        assert!(line.backspace());
        assert_eq!(line.as_str(), "aé");
        assert!(line.backspace());
        assert_eq!(line.as_str(), "a");
    }

    #[test]
    fn backspace_at_the_start_does_nothing() {
        let mut line = line("a");
        line.home();
        assert!(!line.backspace());
        assert_eq!(line.as_str(), "a");
    }

    #[test]
    fn delete_removes_the_char_under_the_cursor() {
        let mut line = line("a😀b");
        line.home();
        line.right();
        assert!(line.delete());
        assert_eq!(line.as_str(), "ab");
        assert_eq!(line.cursor(), 1);
    }

    #[test]
    fn delete_at_the_end_does_nothing() {
        let mut line = line("a");
        assert!(!line.delete());
    }

    #[test]
    fn left_and_right_step_over_whole_chars() {
        let mut line = line("é😀");
        assert!(line.left());
        assert_eq!(line.cursor(), 2);
        assert!(line.left());
        assert_eq!(line.cursor(), 0);
        assert!(!line.left());
        assert!(line.right());
        assert_eq!(line.cursor(), 2);
        assert!(line.right());
        assert!(!line.right());
        assert_eq!(line.cursor(), line.len());
    }

    #[test]
    fn home_and_end_report_whether_they_moved() {
        let mut line = line("abc");
        assert!(!line.end());
        assert!(line.home());
        assert!(!line.home());
        assert!(line.end());
    }

    #[test]
    fn kill_to_end_keeps_the_head() {
        let mut line = line("hello world");
        line.home();
        for _ in 0..5 {
            line.right();
        }
        assert!(line.kill_to_end());
        assert_eq!(line.as_str(), "hello");
        assert!(!line.kill_to_end());
    }

    #[test]
    fn kill_to_start_keeps_the_tail() {
        let mut line = line("hello world");
        line.home();
        for _ in 0..6 {
            line.right();
        }
        assert!(line.kill_to_start());
        assert_eq!(line.as_str(), "world");
        assert_eq!(line.cursor(), 0);
        assert!(!line.kill_to_start());
    }

    #[test]
    fn kill_word_takes_trailing_blanks_and_the_word() {
        let mut line = line("echo hello  ");
        assert!(line.kill_word());
        assert_eq!(line.as_str(), "echo ");
        assert!(line.kill_word());
        assert_eq!(line.as_str(), "");
        assert!(!line.kill_word());
    }

    #[test]
    fn kill_word_works_on_multibyte_words_and_mid_line() {
        let mut line = line("a ação b");
        line.left();
        line.left();
        assert!(line.kill_word());
        assert_eq!(line.as_str(), "a  b");
        assert_eq!(line.cursor(), 2);
    }

    #[test]
    fn set_truncates_on_a_char_boundary() {
        let mut line = LineBuffer::<4>::new();
        line.set("abé€");
        assert_eq!(line.as_str(), "abé");
        assert_eq!(line.cursor(), 4);
    }

    #[test]
    fn char_counts_ignore_byte_width() {
        let mut line = line("aé€😀");
        assert_eq!((line.char_len(), line.char_cursor()), (4, 4));
        line.left();
        assert_eq!(line.char_cursor(), 3);
    }

    #[test]
    fn clear_empties_the_line() {
        let mut line = line("abc");
        line.clear();
        assert!(line.is_empty());
        assert_eq!(line.cursor(), 0);
    }
}
