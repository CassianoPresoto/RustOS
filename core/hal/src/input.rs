//! Porta de entrada: bytes (serial) e teclas físicas (teclado).

/// Fonte de bytes sem bloqueio: `None` quando não há nada esperando.
pub trait ByteSource {
    fn read_byte(&mut self) -> Option<u8>;
}

/// Tecla física pelo código de uso do HID USB (página 0x07): independe do
/// teclado (PS/2, USB, virtio) e do layout. O layout traduz depois.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyCode(pub u8);

impl KeyCode {
    pub const A: Self = Self(0x04);
    pub const Z: Self = Self(0x1D);
    pub const DIGIT_1: Self = Self(0x1E);
    pub const DIGIT_0: Self = Self(0x27);
    pub const ENTER: Self = Self(0x28);
    pub const ESCAPE: Self = Self(0x29);
    pub const BACKSPACE: Self = Self(0x2A);
    pub const TAB: Self = Self(0x2B);
    pub const SPACE: Self = Self(0x2C);
    pub const CAPS_LOCK: Self = Self(0x39);
    pub const F1: Self = Self(0x3A);
    pub const F12: Self = Self(0x45);
    pub const INSERT: Self = Self(0x49);
    pub const HOME: Self = Self(0x4A);
    pub const PAGE_UP: Self = Self(0x4B);
    pub const DELETE: Self = Self(0x4C);
    pub const END: Self = Self(0x4D);
    pub const PAGE_DOWN: Self = Self(0x4E);
    pub const RIGHT: Self = Self(0x4F);
    pub const LEFT: Self = Self(0x50);
    pub const DOWN: Self = Self(0x51);
    pub const UP: Self = Self(0x52);
    pub const LEFT_CTRL: Self = Self(0xE0);
    pub const LEFT_SHIFT: Self = Self(0xE1);
    pub const LEFT_ALT: Self = Self(0xE2);
    pub const RIGHT_CTRL: Self = Self(0xE4);
    pub const RIGHT_SHIFT: Self = Self(0xE5);
    pub const RIGHT_ALT: Self = Self(0xE6);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub pressed: bool,
}
