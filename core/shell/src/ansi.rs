//! Sequências VT que o shell escreve; as cores usam só a paleta de 256.

pub(crate) const RESET: &str = "\x1b[0m";
pub(crate) const ACCENT: &str = "\x1b[1;38;5;208m";
pub(crate) const LOGO: &str = "\x1b[38;5;208m";
pub(crate) const DIM: &str = "\x1b[2m";
pub(crate) const CLEAR_SCREEN: &str = "\x1b[H\x1b[2J";
pub(crate) const BEL: char = '\x07';
