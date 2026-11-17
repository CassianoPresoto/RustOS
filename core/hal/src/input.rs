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
    pub const MINUS: Self = Self(0x2D);
    pub const EQUAL: Self = Self(0x2E);
    pub const LEFT_BRACKET: Self = Self(0x2F);
    pub const RIGHT_BRACKET: Self = Self(0x30);
    pub const BACKSLASH: Self = Self(0x31);
    pub const NON_US_HASH: Self = Self(0x32);
    pub const SEMICOLON: Self = Self(0x33);
    pub const APOSTROPHE: Self = Self(0x34);
    pub const GRAVE: Self = Self(0x35);
    pub const COMMA: Self = Self(0x36);
    pub const PERIOD: Self = Self(0x37);
    pub const SLASH: Self = Self(0x38);
    pub const CAPS_LOCK: Self = Self(0x39);
    pub const F1: Self = Self(0x3A);
    pub const F12: Self = Self(0x45);
    pub const PRINT_SCREEN: Self = Self(0x46);
    pub const SCROLL_LOCK: Self = Self(0x47);
    pub const PAUSE: Self = Self(0x48);
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
    pub const NUM_LOCK: Self = Self(0x53);
    pub const KP_DIVIDE: Self = Self(0x54);
    pub const KP_MULTIPLY: Self = Self(0x55);
    pub const KP_MINUS: Self = Self(0x56);
    pub const KP_PLUS: Self = Self(0x57);
    pub const KP_ENTER: Self = Self(0x58);
    pub const KP_1: Self = Self(0x59);
    pub const KP_0: Self = Self(0x62);
    pub const KP_PERIOD: Self = Self(0x63);
    pub const NON_US_BACKSLASH: Self = Self(0x64);
    pub const APPLICATION: Self = Self(0x65);
    pub const KP_COMMA: Self = Self(0x85);
    pub const INTERNATIONAL1: Self = Self(0x87);
    pub const INTERNATIONAL2: Self = Self(0x88);
    pub const INTERNATIONAL3: Self = Self(0x89);
    pub const INTERNATIONAL4: Self = Self(0x8A);
    pub const INTERNATIONAL5: Self = Self(0x8B);
    pub const LEFT_CTRL: Self = Self(0xE0);
    pub const LEFT_SHIFT: Self = Self(0xE1);
    pub const LEFT_ALT: Self = Self(0xE2);
    pub const LEFT_GUI: Self = Self(0xE3);
    pub const RIGHT_CTRL: Self = Self(0xE4);
    pub const RIGHT_SHIFT: Self = Self(0xE5);
    pub const RIGHT_ALT: Self = Self(0xE6);
    pub const RIGHT_GUI: Self = Self(0xE7);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub pressed: bool,
}
