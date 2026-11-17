//! Porta de vídeo: o framebuffer linear que o firmware entrega, já mapeado.

use crate::VirtAddr;

/// Posição e largura, em bits, de um canal de cor dentro do pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Channel {
    pub shift: u8,
    pub size: u8,
}

/// Framebuffer de 32 bits por pixel, acessível em `address` por `pitch * height` bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramebufferInfo {
    pub address: VirtAddr,
    pub width: u32,
    pub height: u32,
    /// Bytes por linha; pode ser maior que `width * 4`.
    pub pitch: u32,
    pub red: Channel,
    pub green: Channel,
    pub blue: Channel,
}

impl FramebufferInfo {
    pub const BYTES_PER_PIXEL: u32 = 4;

    /// Pixels por linha, contando o enchimento do fim da linha.
    pub const fn stride_pixels(&self) -> u32 {
        self.pitch / Self::BYTES_PER_PIXEL
    }

    pub const fn len_bytes(&self) -> u64 {
        self.pitch as u64 * self.height as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(width: u32, pitch: u32, height: u32) -> FramebufferInfo {
        let channel = Channel { shift: 0, size: 8 };
        FramebufferInfo {
            address: VirtAddr::new(0),
            width,
            height,
            pitch,
            red: channel,
            green: channel,
            blue: channel,
        }
    }

    #[test]
    fn stride_counts_line_padding() {
        assert_eq!(info(1280, 1280 * 4, 800).stride_pixels(), 1280);
        assert_eq!(info(1366, 1376 * 4, 768).stride_pixels(), 1376);
    }

    #[test]
    fn length_uses_pitch_not_width() {
        assert_eq!(info(1366, 1376 * 4, 768).len_bytes(), 1376 * 4 * 768);
    }

    #[test]
    fn length_does_not_overflow_u32() {
        assert_eq!(
            info(u32::MAX / 4, u32::MAX, 2).len_bytes(),
            u64::from(u32::MAX) * 2
        );
    }
}
