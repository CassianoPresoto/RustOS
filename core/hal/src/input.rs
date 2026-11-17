//! Porta de entrada de bytes: serial hoje, teclado depois.

/// Fonte de bytes sem bloqueio: `None` quando não há nada esperando.
pub trait ByteSource {
    fn read_byte(&mut self) -> Option<u8>;
}
