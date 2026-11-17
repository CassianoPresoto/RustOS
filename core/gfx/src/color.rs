//! Cor RGB e o formato do pixel no framebuffer.

/// Cor de 24 bits, independente do formato do framebuffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// `0xRRGGBB`; o byte mais alto é ignorado.
    pub const fn hex(value: u32) -> Self {
        let [_, r, g, b] = value.to_be_bytes();
        Self { r, g, b }
    }
}

/// Pixel já no formato do framebuffer: o caminho quente só copia `u32`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pixel(pub u32);

/// Onde cada canal de 8 bits mora no pixel de 32 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelFormat {
    red: u8,
    green: u8,
    blue: u8,
}

impl PixelFormat {
    /// O formato mais comum do GOP: `0x00RRGGBB` em little-endian.
    pub const BGRX: Self = Self {
        red: 16,
        green: 8,
        blue: 0,
    };

    /// `None` se um canal sai do pixel ou se dois canais se sobrepõem.
    pub const fn new(red_shift: u8, green_shift: u8, blue_shift: u8) -> Option<Self> {
        if red_shift > 24 || green_shift > 24 || blue_shift > 24 {
            return None;
        }
        let (red, green, blue) = (
            0xFF_u32 << red_shift,
            0xFF_u32 << green_shift,
            0xFF_u32 << blue_shift,
        );
        if red & green != 0 || red & blue != 0 || green & blue != 0 {
            return None;
        }
        Some(Self {
            red: red_shift,
            green: green_shift,
            blue: blue_shift,
        })
    }

    pub const fn pack(self, color: Rgb) -> Pixel {
        Pixel(
            (color.r as u32) << self.red
                | (color.g as u32) << self.green
                | (color.b as u32) << self.blue,
        )
    }

    pub const fn unpack(self, pixel: Pixel) -> Rgb {
        Rgb::new(
            channel(pixel, self.red),
            channel(pixel, self.green),
            channel(pixel, self.blue),
        )
    }
}

const fn channel(pixel: Pixel, shift: u8) -> u8 {
    ((pixel.0 >> shift) & 0xFF) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_ignores_the_top_byte() {
        assert_eq!(Rgb::hex(0xAA12_3456), Rgb::new(0x12, 0x34, 0x56));
    }

    #[test]
    fn bgrx_packs_red_in_the_third_byte() {
        let pixel = PixelFormat::BGRX.pack(Rgb::new(0x12, 0x34, 0x56));
        assert_eq!(pixel, Pixel(0x0012_3456));
    }

    #[test]
    fn rgbx_packs_red_in_the_first_byte() {
        let format = PixelFormat::new(0, 8, 16).expect("canais separados");
        assert_eq!(format.pack(Rgb::new(0x12, 0x34, 0x56)), Pixel(0x0056_3412));
    }

    #[test]
    fn unpack_reverses_pack() {
        let format = PixelFormat::new(24, 0, 8).expect("canais separados");
        let color = Rgb::new(1, 2, 3);
        assert_eq!(format.unpack(format.pack(color)), color);
    }

    #[test]
    fn rejects_overlapping_or_out_of_range_channels() {
        assert_eq!(PixelFormat::new(16, 12, 0), None);
        assert_eq!(PixelFormat::new(25, 8, 0), None);
        assert_eq!(PixelFormat::new(0, 0, 16), None);
    }
}
