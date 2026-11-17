//! Porta de saída de texto do kernel (serial, framebuffer, o que vier).

pub trait Console {
    fn write_bytes(&mut self, bytes: &[u8]);
}
