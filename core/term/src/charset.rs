//! Conjuntos de caracteres G0/G1: ASCII e o desenho de linhas do DEC.

/// Conjunto designado a G0 ou G1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Charset {
    #[default]
    Ascii,
    DecGraphics,
}

/// DEC Special Graphics para U+0060..=U+007E, pela tabela `dec2ucs` do xterm.
const DEC_GRAPHICS: [char; 31] = [
    '\u{25C6}', '\u{2592}', '\u{2409}', '\u{240C}', '\u{240D}', '\u{240A}', '\u{00B0}', '\u{00B1}',
    '\u{2424}', '\u{240B}', '\u{2518}', '\u{2510}', '\u{250C}', '\u{2514}', '\u{253C}', '\u{23BA}',
    '\u{23BB}', '\u{2500}', '\u{23BC}', '\u{23BD}', '\u{251C}', '\u{2524}', '\u{2534}', '\u{252C}',
    '\u{2502}', '\u{2264}', '\u{2265}', '\u{03C0}', '\u{2260}', '\u{00A3}', '\u{00B7}',
];

impl Charset {
    /// O que o byte `c` imprime com este conjunto em GL; só ASCII muda.
    pub(crate) fn map(self, c: char) -> char {
        match (self, c) {
            (Charset::DecGraphics, '\u{5F}') => ' ',
            (Charset::DecGraphics, '\u{60}'..='\u{7E}') => DEC_GRAPHICS[c as usize - 0x60],
            _ => c,
        }
    }

    /// O conjunto que `ESC ( final` designa; `0` é o desenho de linhas, o resto vira ASCII.
    pub(crate) fn from_final(byte: u8) -> Self {
        if byte == b'0' {
            Charset::DecGraphics
        } else {
            Charset::Ascii
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_maps_to_itself() {
        for c in (0u8..0x80).map(char::from) {
            assert_eq!(Charset::Ascii.map(c), c);
        }
    }

    #[test]
    fn graphics_maps_the_box_letters() {
        let map = |c| Charset::DecGraphics.map(c);
        let expected = [
            ('q', '─'),
            ('x', '│'),
            ('l', '┌'),
            ('k', '┐'),
            ('m', '└'),
            ('j', '┘'),
            ('t', '├'),
            ('u', '┤'),
            ('v', '┴'),
            ('w', '┬'),
            ('n', '┼'),
        ];
        for (from, to) in expected {
            assert_eq!(map(from), to, "{from}");
        }
    }

    #[test]
    fn graphics_maps_the_rest_of_the_set() {
        let map = |c| Charset::DecGraphics.map(c);
        assert_eq!(map('`'), '◆');
        assert_eq!(map('a'), '▒');
        assert_eq!(map('f'), '°');
        assert_eq!(map('g'), '±');
        assert_eq!(map('o'), '\u{23BA}');
        assert_eq!(map('s'), '\u{23BD}');
        assert_eq!(map('y'), '≤');
        assert_eq!(map('z'), '≥');
        assert_eq!(map('{'), 'π');
        assert_eq!(map('|'), '≠');
        assert_eq!(map('}'), '£');
        assert_eq!(map('~'), '·');
        assert_eq!(map('_'), ' ');
    }

    #[test]
    fn graphics_leaves_everything_else_alone() {
        for c in (0x20u8..0x5F).chain([0x7F]).map(char::from) {
            assert_eq!(Charset::DecGraphics.map(c), c);
        }
        assert_eq!(Charset::DecGraphics.map('é'), 'é');
        assert_eq!(Charset::DecGraphics.map('─'), '─');
    }

    #[test]
    fn only_final_zero_designates_the_graphics_set() {
        assert_eq!(Charset::from_final(b'0'), Charset::DecGraphics);
        for byte in *b"BA12K<" {
            assert_eq!(Charset::from_final(byte), Charset::Ascii);
        }
    }
}
