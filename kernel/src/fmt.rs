//! Texto formatado sem alocação: buffer de capacidade fixa que trunca no limite.

use core::fmt;

#[derive(Debug)]
pub struct Buffer<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Buffer<N> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }

    pub fn as_str(&self) -> &str {
        // Só entra texto inteiro de `&str`, cortado em fronteira de caractere.
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or_default()
    }
}

impl<const N: usize> fmt::Write for Buffer<N> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let room = N - self.len;
        let mut take = text.len().min(room);
        while !text.is_char_boundary(take) {
            take -= 1;
        }
        self.bytes[self.len..self.len + take].copy_from_slice(&text.as_bytes()[..take]);
        self.len += take;
        if take == text.len() {
            Ok(())
        } else {
            Err(fmt::Error)
        }
    }
}
