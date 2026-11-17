//! Célula da grade: o contrato entre o emulador e quem desenha.

/// Atributos de uma célula, em um byte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags(u8);

impl Flags {
    /// Nenhum atributo ligado.
    pub const NONE: Flags = Flags(0);
    /// Negrito (SGR 1). Não muda o índice da cor: o renderizador decide.
    pub const BOLD: Flags = Flags(1);
    /// Intensidade reduzida (SGR 2).
    pub const DIM: Flags = Flags(2);
    /// Itálico (SGR 3).
    pub const ITALIC: Flags = Flags(4);
    /// Sublinhado (SGR 4).
    pub const UNDERLINE: Flags = Flags(8);
    /// Frente e fundo trocados (SGR 7).
    pub const INVERSE: Flags = Flags(16);
    /// Riscado (SGR 9).
    pub const STRIKE: Flags = Flags(32);
    /// A frente é a cor padrão do tema; `Cell::fg` não vale.
    pub const DEFAULT_FG: Flags = Flags(64);
    /// O fundo é a cor padrão do tema; `Cell::bg` não vale.
    pub const DEFAULT_BG: Flags = Flags(128);

    /// Todos os bits de `other` estão ligados.
    pub const fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    /// Liga os bits de `other`.
    pub const fn insert(&mut self, other: Flags) {
        self.0 |= other.0;
    }

    /// Desliga os bits de `other`.
    pub const fn remove(&mut self, other: Flags) {
        self.0 &= !other.0;
    }

    /// União de dois conjuntos de atributos.
    #[must_use]
    pub const fn union(self, other: Flags) -> Flags {
        Flags(self.0 | other.0)
    }

    /// Os bits crus, para quem serializa a célula.
    pub const fn bits(self) -> u8 {
        self.0
    }
}

/// Uma célula da grade: caractere, cores (índice xterm 256) e atributos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Caractere exibido.
    pub ch: char,
    /// Índice da frente na paleta xterm 256; vale se `DEFAULT_FG` estiver desligado.
    pub fg: u8,
    /// Índice do fundo na paleta xterm 256; vale se `DEFAULT_BG` estiver desligado.
    pub bg: u8,
    /// Atributos da célula.
    pub flags: Flags,
}

const _: () = assert!(size_of::<Cell>() == 8);

impl Cell {
    /// Espaço com as cores padrão: o conteúdo de uma tela recém-criada.
    pub const BLANK: Cell = Cell {
        ch: ' ',
        fg: 7,
        bg: 0,
        flags: Flags::DEFAULT_FG.union(Flags::DEFAULT_BG),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_takes_eight_bytes() {
        assert_eq!(size_of::<Cell>(), 8);
    }

    #[test]
    fn blank_cell_is_a_space_with_default_colors() {
        assert_eq!(Cell::BLANK.ch, ' ');
        assert!(Cell::BLANK.flags.contains(Flags::DEFAULT_FG));
        assert!(Cell::BLANK.flags.contains(Flags::DEFAULT_BG));
        assert!(!Cell::BLANK.flags.contains(Flags::BOLD));
    }

    #[test]
    fn flags_insert_and_remove_touch_only_their_bits() {
        let mut flags = Flags::NONE;
        flags.insert(Flags::BOLD);
        flags.insert(Flags::UNDERLINE);
        assert!(flags.contains(Flags::BOLD) && flags.contains(Flags::UNDERLINE));
        flags.remove(Flags::BOLD);
        assert!(!flags.contains(Flags::BOLD) && flags.contains(Flags::UNDERLINE));
        flags.remove(Flags::BOLD);
        assert_eq!(flags, Flags::UNDERLINE);
    }

    #[test]
    fn flags_contains_requires_every_bit() {
        let both = Flags::BOLD.union(Flags::ITALIC);
        assert!(both.contains(Flags::BOLD));
        assert!(both.contains(both));
        assert!(!Flags::BOLD.contains(both));
        assert!(Flags::NONE.contains(Flags::NONE));
    }

    #[test]
    fn flags_are_distinct_single_bits() {
        let all = [
            Flags::BOLD,
            Flags::DIM,
            Flags::ITALIC,
            Flags::UNDERLINE,
            Flags::INVERSE,
            Flags::STRIKE,
            Flags::DEFAULT_FG,
            Flags::DEFAULT_BG,
        ];
        let mut seen = 0u8;
        for flag in all {
            assert_eq!(flag.bits().count_ones(), 1);
            assert_eq!(seen & flag.bits(), 0);
            seen |= flag.bits();
        }
        assert_eq!(seen, 0xFF);
    }

    #[test]
    fn flags_union_combines_bits() {
        assert_eq!(Flags::BOLD.union(Flags::DIM).bits(), 3);
        assert_eq!(Flags::default(), Flags::NONE);
    }
}
