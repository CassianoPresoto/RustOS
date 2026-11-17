//! Decodificador do conjunto 1 de scancodes (o que o i8042 entrega com a
//! tradução ligada) para `KeyCode` do HID. Função pura: roda no host.

use rustos_hal::{KeyCode, KeyEvent};

const PREFIX_EXTENDED: u8 = 0xE0;
const PREFIX_PAUSE: u8 = 0xE1;
const PAUSE_TAIL: u8 = 5;
const BREAK_BIT: u8 = 0x80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    Extended,
    PauseTail(u8),
}

/// Máquina de estados byte a byte; guarda só o prefixo pendente.
#[derive(Clone, Copy, Debug)]
pub struct Set1Decoder {
    state: State,
}

impl Set1Decoder {
    pub const fn new() -> Self {
        Self { state: State::Idle }
    }

    /// Descarta o prefixo pendente (byte com erro de paridade ou timeout).
    pub fn reset(&mut self) {
        self.state = State::Idle;
    }

    /// Consome um byte; devolve o evento quando ele completa uma tecla.
    pub fn feed(&mut self, byte: u8) -> Option<KeyEvent> {
        match self.state {
            State::Idle => match byte {
                PREFIX_EXTENDED => {
                    self.state = State::Extended;
                    None
                }
                PREFIX_PAUSE => {
                    self.state = State::PauseTail(PAUSE_TAIL);
                    None
                }
                _ => event(plain(byte & !BREAK_BIT), byte),
            },
            State::Extended => {
                if byte == PREFIX_EXTENDED {
                    return None;
                }
                self.state = State::Idle;
                event(extended(byte & !BREAK_BIT), byte)
            }
            State::PauseTail(left) => {
                self.state = if left > 1 {
                    State::PauseTail(left - 1)
                } else {
                    State::Idle
                };
                None
            }
        }
    }
}

impl Default for Set1Decoder {
    fn default() -> Self {
        Self::new()
    }
}

fn event(code: Option<u8>, byte: u8) -> Option<KeyEvent> {
    code.map(|code| KeyEvent {
        code: KeyCode(code),
        pressed: byte & BREAK_BIT == 0,
    })
}

/// Q a P, A a L e Z a M: os usos do HID das letras não seguem a ordem do teclado.
const QWERTY_ROW: [u8; 10] = [0x14, 0x1A, 0x08, 0x15, 0x17, 0x1C, 0x18, 0x0C, 0x12, 0x13];
const HOME_ROW: [u8; 9] = [0x04, 0x16, 0x07, 0x09, 0x0A, 0x0B, 0x0D, 0x0E, 0x0F];
const BOTTOM_ROW: [u8; 7] = [0x1D, 0x1B, 0x06, 0x19, 0x05, 0x11, 0x10];

/// Conjunto 1 sem prefixo, pelo make code. Letras e números pelo HID, não pela
/// posição: o layout decide o caractere.
fn plain(make: u8) -> Option<u8> {
    match make {
        0x01 => Some(0x29),
        0x02..=0x0A => Some(0x1E + (make - 0x02)),
        0x0B => Some(0x27),
        0x0C => Some(0x2D),
        0x0D => Some(0x2E),
        0x0E => Some(0x2A),
        0x0F => Some(0x2B),
        0x10..=0x19 => Some(QWERTY_ROW[usize::from(make - 0x10)]),
        0x1A => Some(0x2F),
        0x1B => Some(0x30),
        0x1C => Some(0x28),
        0x1D => Some(0xE0),
        0x1E..=0x26 => Some(HOME_ROW[usize::from(make - 0x1E)]),
        0x27 => Some(0x33),
        0x28 => Some(0x34),
        0x29 => Some(0x35),
        0x2A => Some(0xE1),
        0x2B => Some(0x31),
        0x2C..=0x32 => Some(BOTTOM_ROW[usize::from(make - 0x2C)]),
        0x33 => Some(0x36),
        0x34 => Some(0x37),
        0x35 => Some(0x38),
        0x36 => Some(0xE5),
        0x37 => Some(0x55),
        0x38 => Some(0xE2),
        0x39 => Some(0x2C),
        0x3A => Some(0x39),
        0x3B..=0x44 => Some(0x3A + (make - 0x3B)),
        0x45 => Some(0x53),
        0x46 => Some(0x47),
        0x47..=0x49 => Some(0x5F + (make - 0x47)),
        0x4A => Some(0x56),
        0x4B..=0x4D => Some(0x5C + (make - 0x4B)),
        0x4E => Some(0x57),
        0x4F..=0x51 => Some(0x59 + (make - 0x4F)),
        0x52 => Some(0x62),
        0x53 => Some(0x63),
        0x54 => Some(0x46),
        0x56 => Some(0x64),
        0x57 => Some(0x44),
        0x58 => Some(0x45),
        0x70 => Some(0x88),
        0x73 => Some(0x87),
        0x79 => Some(0x8A),
        0x7B => Some(0x8B),
        0x7D => Some(0x89),
        0x7E => Some(0x85),
        _ => None,
    }
}

/// Conjunto 1 com prefixo `E0`. O `E0 2A`/`E0 36` (shift falso do Print Screen
/// e das teclas de edição com Num Lock) não está aqui: não gera evento.
fn extended(make: u8) -> Option<u8> {
    match make {
        0x1C => Some(0x58),
        0x1D => Some(0xE4),
        0x35 => Some(0x54),
        0x37 => Some(0x46),
        0x38 => Some(0xE6),
        0x46 => Some(0x48),
        0x47 => Some(0x4A),
        0x48 => Some(0x52),
        0x49 => Some(0x4B),
        0x4B => Some(0x50),
        0x4D => Some(0x4F),
        0x4F => Some(0x4D),
        0x50 => Some(0x51),
        0x51 => Some(0x4E),
        0x52 => Some(0x49),
        0x53 => Some(0x4C),
        0x5B => Some(0xE3),
        0x5C => Some(0xE7),
        0x5D => Some(0x65),
        _ => None,
    }
}
