//! Superfície de desenho sobre uma fatia de pixels de 32 bits: o framebuffer,
//! ou um buffer comum no teste. Todo desenho é recortado à superfície.

use crate::color::{Pixel, PixelFormat};

/// Maior ampliação de glifo; a linha expandida mora na pilha.
pub const MAX_SCALE: usize = 8;

const GLYPH_BITS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    pub const fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Recorta a `[0, width) x [0, height)`; `None` se nada sobra.
    pub fn clip(self, width: usize, height: usize) -> Option<Self> {
        let right = self.x.saturating_add(self.width).min(width);
        let bottom = self.y.saturating_add(self.height).min(height);
        (self.x < right && self.y < bottom).then(|| Self {
            x: self.x,
            y: self.y,
            width: right - self.x,
            height: bottom - self.y,
        })
    }
}

#[derive(Debug)]
pub struct Surface<'a> {
    pixels: &'a mut [u32],
    width: usize,
    height: usize,
    stride: usize,
    format: PixelFormat,
}

impl<'a> Surface<'a> {
    /// `stride` é em pixels. `None` se a fatia não cobre a última linha visível.
    pub fn new(
        pixels: &'a mut [u32],
        width: usize,
        height: usize,
        stride: usize,
        format: PixelFormat,
    ) -> Option<Self> {
        if width == 0 || height == 0 || stride < width {
            return None;
        }
        let needed = stride.checked_mul(height - 1)?.checked_add(width)?;
        (pixels.len() >= needed).then_some(Self {
            pixels,
            width,
            height,
            stride,
            format,
        })
    }

    pub const fn width(&self) -> usize {
        self.width
    }

    pub const fn height(&self) -> usize {
        self.height
    }

    pub const fn format(&self) -> PixelFormat {
        self.format
    }

    pub fn pixel(&self, x: usize, y: usize) -> Option<Pixel> {
        (x < self.width && y < self.height).then(|| Pixel(self.pixels[y * self.stride + x]))
    }

    pub fn fill(&mut self, color: Pixel) {
        self.fill_rect(Rect::new(0, 0, self.width, self.height), color);
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Pixel) {
        let Some(rect) = rect.clip(self.width, self.height) else {
            return;
        };
        for y in rect.y..rect.y + rect.height {
            let start = y * self.stride + rect.x;
            self.pixels[start..start + rect.width].fill(color.0);
        }
    }

    /// Desenha um glifo de 8 colunas (bit 7 à esquerda), ampliado `scale` vezes.
    /// Escreve linha a linha, em ordem, sem ler o framebuffer.
    pub fn draw_glyph(
        &mut self,
        x: usize,
        y: usize,
        rows: &[u8],
        scale: usize,
        fg: Pixel,
        bg: Pixel,
    ) {
        let scale = scale.min(MAX_SCALE);
        let full = GLYPH_BITS * scale;
        let area = Rect::new(x, y, full, rows.len() * scale);
        let Some(visible) = area.clip(self.width, self.height) else {
            return;
        };
        let mut line = [0_u32; GLYPH_BITS * MAX_SCALE];
        for (row, &bits) in rows.iter().enumerate() {
            let top = y + row * scale;
            if top >= visible.y + visible.height {
                break;
            }
            for (column, chunk) in line[..full].chunks_exact_mut(scale).enumerate() {
                let lit = bits & (0x80 >> column) != 0;
                chunk.fill(if lit { fg.0 } else { bg.0 });
            }
            let span = &line[..visible.width];
            for target_y in top..(top + scale).min(visible.y + visible.height) {
                let start = target_y * self.stride + x;
                self.pixels[start..start + visible.width].copy_from_slice(span);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BG: Pixel = Pixel(0);
    const FG: Pixel = Pixel(0xFF_FFFF);

    fn buffer(stride: usize, height: usize) -> Vec<u32> {
        vec![0xDEAD; stride * height]
    }

    fn surface(pixels: &mut [u32], width: usize, height: usize, stride: usize) -> Surface<'_> {
        Surface::new(pixels, width, height, stride, PixelFormat::BGRX).expect("fatia suficiente")
    }

    #[test]
    fn rejects_short_slice_zero_size_and_narrow_stride() {
        let mut pixels = buffer(4, 4);
        assert!(Surface::new(&mut pixels, 4, 5, 4, PixelFormat::BGRX).is_none());
        assert!(Surface::new(&mut pixels, 0, 4, 4, PixelFormat::BGRX).is_none());
        assert!(Surface::new(&mut pixels, 5, 2, 4, PixelFormat::BGRX).is_none());
    }

    #[test]
    fn reports_its_geometry_and_format() {
        let mut pixels = buffer(8, 3);
        let surface = surface(&mut pixels, 6, 3, 8);
        assert_eq!((surface.width(), surface.height()), (6, 3));
        assert_eq!(surface.format(), PixelFormat::BGRX);
        assert_eq!(surface.pixel(6, 0), None);
        assert_eq!(surface.pixel(0, 3), None);
    }

    #[test]
    fn last_row_does_not_need_padding() {
        let mut pixels = vec![0; 8 * 2 + 4];
        assert!(Surface::new(&mut pixels, 4, 3, 8, PixelFormat::BGRX).is_some());
    }

    #[test]
    fn fill_rect_is_clipped_and_respects_stride() {
        let mut pixels = buffer(6, 3);
        let mut surface = surface(&mut pixels, 4, 3, 6);
        surface.fill_rect(Rect::new(2, 1, 10, 10), FG);
        assert_eq!(surface.pixel(1, 1), Some(Pixel(0xDEAD)));
        assert_eq!(surface.pixel(3, 2), Some(FG));
        assert_eq!(pixels[6 + 4], 0xDEAD, "enchimento da linha intacto");
    }

    #[test]
    fn fill_rect_outside_does_nothing() {
        let mut pixels = buffer(4, 4);
        let mut surface = surface(&mut pixels, 4, 4, 4);
        surface.fill_rect(Rect::new(4, 0, 2, 2), FG);
        surface.fill_rect(Rect::new(usize::MAX, usize::MAX, 9, 9), FG);
        assert!(pixels.iter().all(|&pixel| pixel == 0xDEAD));
    }

    #[test]
    fn glyph_bit_seven_is_the_leftmost_pixel() {
        let mut pixels = buffer(8, 2);
        let mut surface = surface(&mut pixels, 8, 2, 8);
        surface.draw_glyph(0, 0, &[0b1000_0001, 0b0100_0000], 1, FG, BG);
        assert_eq!(surface.pixel(0, 0), Some(FG));
        assert_eq!(surface.pixel(1, 0), Some(BG));
        assert_eq!(surface.pixel(7, 0), Some(FG));
        assert_eq!(surface.pixel(1, 1), Some(FG));
    }

    #[test]
    fn scaled_glyph_covers_a_square_per_bit() {
        let mut pixels = buffer(16, 4);
        let mut surface = surface(&mut pixels, 16, 4, 16);
        surface.draw_glyph(0, 0, &[0b1000_0000, 0], 2, FG, BG);
        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            assert_eq!(surface.pixel(x, y), Some(FG));
        }
        assert_eq!(surface.pixel(2, 0), Some(BG));
        assert_eq!(surface.pixel(0, 2), Some(BG));
    }

    #[test]
    fn glyph_is_clipped_at_right_and_bottom_edges() {
        let mut pixels = buffer(10, 3);
        let mut surface = surface(&mut pixels, 10, 3, 10);
        surface.draw_glyph(6, 1, &[0xFF; 16], 1, FG, BG);
        assert_eq!(surface.pixel(9, 2), Some(FG));
        assert_eq!(surface.pixel(5, 1), Some(Pixel(0xDEAD)));
        assert_eq!(surface.pixel(6, 0), Some(Pixel(0xDEAD)));
    }

    #[test]
    fn scale_is_capped_and_zero_draws_nothing() {
        let mut pixels = buffer(80, 10);
        let mut surface = surface(&mut pixels, 80, 10, 80);
        surface.draw_glyph(0, 0, &[0x80], 0, FG, BG);
        assert_eq!(surface.pixel(0, 0), Some(Pixel(0xDEAD)));
        surface.draw_glyph(0, 0, &[0x80], 100, FG, BG);
        assert_eq!(surface.pixel(MAX_SCALE - 1, MAX_SCALE - 1), Some(FG));
        assert_eq!(surface.pixel(MAX_SCALE, 0), Some(BG));
    }

    #[test]
    fn rect_clip_handles_overflow() {
        let rect = Rect::new(usize::MAX - 1, 0, 10, 1);
        assert_eq!(
            rect.clip(usize::MAX, 1),
            Some(Rect::new(usize::MAX - 1, 0, 1, 1))
        );
    }
}
