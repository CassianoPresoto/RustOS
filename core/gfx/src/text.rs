//! Estilos de texto aplicados ao bitmap do glifo antes de desenhar.

/// Negrito engrossa o traço para a direita: exige a última coluna livre no glifo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub underline: bool,
    pub strike: bool,
}

impl Style {
    pub const PLAIN: Self = Self {
        bold: false,
        underline: false,
        strike: false,
    };

    /// Sublinhado na penúltima linha; tachado no meio da altura-x da fonte 8x16.
    pub fn apply<const N: usize>(self, glyph: &[u8; N]) -> [u8; N] {
        let mut rows = *glyph;
        if self.bold {
            for row in &mut rows {
                *row |= *row >> 1;
            }
        }
        if self.underline && N >= 2 {
            rows[N - 2] = 0xFF;
        }
        if self.strike && N > 0 {
            rows[N * 9 / 16] = 0xFF;
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_keeps_the_glyph() {
        let glyph = [0b0101_0000; 16];
        assert_eq!(Style::PLAIN.apply(&glyph), glyph);
    }

    #[test]
    fn bold_thickens_strokes_to_the_right() {
        let style = Style {
            bold: true,
            ..Style::PLAIN
        };
        assert_eq!(style.apply(&[0b0100_0010]), [0b0110_0011]);
    }

    #[test]
    fn underline_and_strike_fill_whole_rows() {
        let style = Style {
            underline: true,
            strike: true,
            ..Style::PLAIN
        };
        let rows = style.apply(&[0_u8; 16]);
        assert_eq!(rows[14], 0xFF);
        assert_eq!(rows[9], 0xFF);
        assert_eq!(rows.iter().filter(|&&row| row != 0).count(), 2);
    }

    #[test]
    fn empty_glyph_is_safe() {
        let style = Style {
            bold: true,
            underline: true,
            strike: true,
        };
        assert_eq!(style.apply(&[]), []);
    }
}
