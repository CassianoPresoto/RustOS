//! Decodificador de entrada VT: bytes do teclado ou da serial viram teclas.

/// Tecla já decodificada.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    KillToEnd,
    KillToStart,
    KillWord,
    ClearScreen,
    Interrupt,
    EndOfFile,
}

#[derive(Clone, Copy, Debug)]
enum State {
    Ground,
    Escape,
    Ss3,
    /// `extra`: veio `;`, intermediário ou marcador privado; deixa de ser tecla simples.
    Csi {
        first: u16,
        digits: bool,
        extra: bool,
    },
    Utf8 {
        bytes: [u8; 4],
        have: usize,
        total: usize,
    },
}

/// Máquina de estados byte a byte. Sequência desconhecida é consumida inteira.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Decoder {
    state: State,
    after_cr: bool,
}

impl Decoder {
    pub(crate) const fn new() -> Self {
        Self {
            state: State::Ground,
            after_cr: false,
        }
    }

    /// Consome um byte; devolve a tecla quando ele a completa.
    pub(crate) fn push(&mut self, byte: u8) -> Option<Key> {
        let after_cr = core::mem::take(&mut self.after_cr);
        match self.state {
            State::Ground => self.ground(byte, after_cr),
            State::Escape => self.escape(byte),
            State::Ss3 => {
                self.state = State::Ground;
                letter_key(byte)
            }
            State::Csi {
                first,
                digits,
                extra,
            } => self.csi(byte, first, digits, extra),
            State::Utf8 { bytes, have, total } => self.utf8(byte, bytes, have, total),
        }
    }

    fn ground(&mut self, byte: u8, after_cr: bool) -> Option<Key> {
        match byte {
            b'\r' => {
                self.after_cr = true;
                Some(Key::Enter)
            }
            b'\n' => (!after_cr).then_some(Key::Enter),
            b'\t' => Some(Key::Tab),
            0x08 | 0x7f => Some(Key::Backspace),
            0x01 => Some(Key::Home),
            0x02 => Some(Key::Left),
            0x03 => Some(Key::Interrupt),
            0x04 => Some(Key::EndOfFile),
            0x05 => Some(Key::End),
            0x06 => Some(Key::Right),
            0x0b => Some(Key::KillToEnd),
            0x0c => Some(Key::ClearScreen),
            0x0e => Some(Key::Down),
            0x10 => Some(Key::Up),
            0x15 => Some(Key::KillToStart),
            0x17 => Some(Key::KillWord),
            0x1b => {
                self.state = State::Escape;
                None
            }
            0x20..=0x7e => Some(Key::Char(char::from(byte))),
            0xc2..=0xdf => self.start_utf8(byte, 2),
            0xe0..=0xef => self.start_utf8(byte, 3),
            0xf0..=0xf4 => self.start_utf8(byte, 4),
            _ => None,
        }
    }

    fn start_utf8(&mut self, lead: u8, total: usize) -> Option<Key> {
        let mut bytes = [0; 4];
        bytes[0] = lead;
        self.state = State::Utf8 {
            bytes,
            have: 1,
            total,
        };
        None
    }

    fn utf8(&mut self, byte: u8, mut bytes: [u8; 4], have: usize, total: usize) -> Option<Key> {
        if byte & 0xc0 != 0x80 {
            self.state = State::Ground;
            return self.ground(byte, false);
        }
        bytes[have] = byte;
        let have = have + 1;
        if have < total {
            self.state = State::Utf8 { bytes, have, total };
            return None;
        }
        self.state = State::Ground;
        core::str::from_utf8(&bytes[..total])
            .ok()
            .and_then(|text| text.chars().next())
            .filter(|c| !c.is_control())
            .map(Key::Char)
    }

    fn escape(&mut self, byte: u8) -> Option<Key> {
        self.state = match byte {
            b'[' => State::Csi {
                first: 0,
                digits: false,
                extra: false,
            },
            b'O' => State::Ss3,
            0x1b => State::Escape,
            _ => State::Ground,
        };
        None
    }

    fn csi(&mut self, byte: u8, mut first: u16, mut digits: bool, mut extra: bool) -> Option<Key> {
        match byte {
            b'0'..=b'9' => {
                if !extra {
                    first = first
                        .saturating_mul(10)
                        .saturating_add(u16::from(byte - b'0'));
                    digits = true;
                }
            }
            b';' | 0x20..=0x2f | 0x3a..=0x3f => extra = true,
            0x40..=0x7e => {
                self.state = State::Ground;
                let param = digits.then_some(first);
                return if extra { None } else { csi_key(byte, param) };
            }
            0x1b => {
                self.state = State::Escape;
                return None;
            }
            _ => {
                self.state = State::Ground;
                return self.ground(byte, false);
            }
        }
        self.state = State::Csi {
            first,
            digits,
            extra,
        };
        None
    }
}

/// Setas, Home e End: a letra final é a mesma em `ESC [` e em `ESC O`.
fn letter_key(byte: u8) -> Option<Key> {
    match byte {
        b'A' => Some(Key::Up),
        b'B' => Some(Key::Down),
        b'C' => Some(Key::Right),
        b'D' => Some(Key::Left),
        b'H' => Some(Key::Home),
        b'F' => Some(Key::End),
        _ => None,
    }
}

fn csi_key(final_byte: u8, param: Option<u16>) -> Option<Key> {
    match (final_byte, param) {
        (b'~', Some(1 | 7)) => Some(Key::Home),
        (b'~', Some(4 | 8)) => Some(Key::End),
        (b'~', Some(3)) => Some(Key::Delete),
        (b'~', _) => None,
        (_, None | Some(1)) => letter_key(final_byte),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(bytes: &[u8]) -> Vec<Key> {
        let mut decoder = Decoder::new();
        bytes.iter().filter_map(|&b| decoder.push(b)).collect()
    }

    #[test]
    fn printable_ascii_is_a_char() {
        assert_eq!(
            decode(b"a~ "),
            [Key::Char('a'), Key::Char('~'), Key::Char(' ')]
        );
    }

    #[test]
    fn multibyte_utf8_is_one_char() {
        assert_eq!(
            decode("é€😀".as_bytes()),
            [Key::Char('é'), Key::Char('€'), Key::Char('😀')]
        );
    }

    #[test]
    fn utf8_split_across_pushes_completes_on_the_last_byte() {
        let mut decoder = Decoder::new();
        assert_eq!(decoder.push(0xe2), None);
        assert_eq!(decoder.push(0x82), None);
        assert_eq!(decoder.push(0xac), Some(Key::Char('€')));
    }

    #[test]
    fn broken_utf8_is_dropped_and_the_next_byte_is_kept() {
        assert_eq!(decode(&[0xe2, 0x82, b'x']), [Key::Char('x')]);
        assert_eq!(decode(&[0xc3, b'\r']), [Key::Enter]);
    }

    #[test]
    fn stray_continuation_and_invalid_lead_bytes_are_dropped() {
        assert_eq!(
            decode(&[0x80, 0xbf, 0xc0, 0xc1, 0xf5, 0xff, b'z']),
            [Key::Char('z')]
        );
    }

    #[test]
    fn overlong_and_surrogate_encodings_are_dropped() {
        assert_eq!(decode(&[0xe0, 0x80, 0x80]).len(), 0);
        assert_eq!(decode(&[0xed, 0xa0, 0x80]).len(), 0);
    }

    #[test]
    fn c1_control_characters_are_dropped() {
        assert_eq!(decode("\u{85}".as_bytes()).len(), 0);
    }

    #[test]
    fn cr_and_lf_each_submit() {
        assert_eq!(decode(b"\r"), [Key::Enter]);
        assert_eq!(decode(b"\n"), [Key::Enter]);
        assert_eq!(decode(b"\n\r"), [Key::Enter, Key::Enter]);
    }

    #[test]
    fn cr_lf_counts_once() {
        assert_eq!(decode(b"\r\n"), [Key::Enter]);
        assert_eq!(decode(b"\r\n\n"), [Key::Enter, Key::Enter]);
    }

    #[test]
    fn cr_lf_after_another_byte_still_counts_once() {
        assert_eq!(decode(b"\r\r\n"), [Key::Enter, Key::Enter]);
    }

    #[test]
    fn del_and_bs_are_backspace() {
        assert_eq!(decode(&[0x7f, 0x08]), [Key::Backspace, Key::Backspace]);
    }

    #[test]
    fn control_keys_map_to_editing_keys() {
        assert_eq!(
            decode(&[1, 2, 3, 4, 5, 6, 9, 0x0b, 0x0c, 0x0e, 0x10, 0x15, 0x17]),
            [
                Key::Home,
                Key::Left,
                Key::Interrupt,
                Key::EndOfFile,
                Key::End,
                Key::Right,
                Key::Tab,
                Key::KillToEnd,
                Key::ClearScreen,
                Key::Down,
                Key::Up,
                Key::KillToStart,
                Key::KillWord,
            ]
        );
    }

    #[test]
    fn other_control_bytes_are_ignored() {
        assert_eq!(decode(&[0, 7, 0x0f, 0x11, 0x18, 0x1a, 0x1f]).len(), 0);
    }

    #[test]
    fn arrows_in_normal_and_application_mode() {
        assert_eq!(
            decode(b"\x1b[A\x1b[B\x1b[C\x1b[D"),
            [Key::Up, Key::Down, Key::Right, Key::Left]
        );
        assert_eq!(
            decode(b"\x1bOA\x1bOB\x1bOC\x1bOD"),
            [Key::Up, Key::Down, Key::Right, Key::Left]
        );
    }

    #[test]
    fn home_and_end_in_every_form() {
        assert_eq!(decode(b"\x1b[H\x1b[1~\x1b[7~\x1bOH"), [Key::Home; 4]);
        assert_eq!(decode(b"\x1b[F\x1b[4~\x1b[8~\x1bOF"), [Key::End; 4]);
    }

    #[test]
    fn delete_is_csi_3_tilde() {
        assert_eq!(decode(b"\x1b[3~"), [Key::Delete]);
    }

    #[test]
    fn explicit_one_parameter_still_moves() {
        assert_eq!(decode(b"\x1b[1A"), [Key::Up]);
    }

    #[test]
    fn unknown_csi_is_consumed_whole() {
        assert_eq!(decode(b"\x1b[5~x"), [Key::Char('x')]);
        assert_eq!(decode(b"\x1b[200~x\x1b[201~"), [Key::Char('x')]);
        assert_eq!(decode(b"\x1b[?1;2cy"), [Key::Char('y')]);
        assert_eq!(decode(b"\x1b[1;5Cy"), [Key::Char('y')]);
        assert_eq!(decode(b"\x1b[2Jy"), [Key::Char('y')]);
    }

    #[test]
    fn csi_with_huge_parameter_does_not_overflow() {
        assert_eq!(decode(b"\x1b[99999999999~x"), [Key::Char('x')]);
    }

    #[test]
    fn unknown_ss3_and_alt_keys_are_consumed() {
        assert_eq!(decode(b"\x1bOPx"), [Key::Char('x')]);
        assert_eq!(decode(b"\x1bbx"), [Key::Char('x')]);
        assert_eq!(decode(b"\x1b\x7fx"), [Key::Char('x')]);
    }

    #[test]
    fn escape_restarts_inside_a_sequence() {
        assert_eq!(decode(b"\x1b\x1b[A"), [Key::Up]);
        assert_eq!(decode(b"\x1b[1\x1b[B"), [Key::Down]);
    }

    #[test]
    fn control_byte_aborts_csi_and_is_handled() {
        assert_eq!(decode(b"\x1b[1\r"), [Key::Enter]);
        assert_eq!(decode(b"\x1b[1\x18x"), [Key::Char('x')]);
    }
}
