//! Texto para o console de depuração: a linha é montada inteira num buffer e vai numa
//! `debug_write` só, porque o kernel põe `[user] ` na frente de cada chamada.

use core::fmt::{self, Write};

use rustos_hal::abi::DEBUG_WRITE_MAX;

const CAPACITY: usize = DEBUG_WRITE_MAX as usize;

/// Um buffer de uma `debug_write`: o que passa de `DEBUG_WRITE_MAX` bytes é cortado.
#[derive(Debug)]
pub struct LineBuffer {
    bytes: [u8; CAPACITY],
    len: usize,
}

impl LineBuffer {
    pub const fn new() -> Self {
        Self {
            bytes: [0; CAPACITY],
            len: 0,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl Default for LineBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Write for LineBuffer {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let room = CAPACITY - self.len;
        let take = text.len().min(room);
        self.bytes[self.len..self.len + take].copy_from_slice(&text.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}

/// Formata `args` e escreve numa `debug_write`. Erro do console não tem a quem ser dito: some.
pub fn print_line(args: fmt::Arguments<'_>) {
    let mut line = LineBuffer::new();
    let _ = line.write_fmt(args);
    #[cfg(target_os = "none")]
    let _ = crate::sys::debug_write(line.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_kept_whole_and_cut_at_the_limit() {
        let mut line = LineBuffer::new();
        write!(line, "init: {}", 42).unwrap();
        assert_eq!(line.as_bytes(), b"init: 42");
        let long = "x".repeat(CAPACITY + 10);
        let mut line = LineBuffer::new();
        line.write_str(&long).unwrap();
        assert_eq!(line.as_bytes().len(), CAPACITY);
        line.write_str("more").unwrap();
        assert_eq!(line.as_bytes().len(), CAPACITY);
    }

    #[test]
    fn pieces_of_one_message_stay_in_one_buffer() {
        let mut line = LineBuffer::new();
        write!(line, "a {:#x} b {}", 255, "c").unwrap();
        assert_eq!(line.as_bytes(), b"a 0xff b c");
    }
}
