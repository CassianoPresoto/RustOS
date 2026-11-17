//! Tecla do teclado para os bytes que o programa no terminal espera ler.

/// Tecla entregue pelo teclado ao terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Caractere imprimível, enviado em UTF-8.
    Char(char),
    /// Letra ou símbolo com Ctrl: `Ctrl('c')` vira 0x03.
    Ctrl(char),
    /// Enter, enviado como CR.
    Enter,
    /// Backspace, enviado como DEL (0x7F), como o xterm.
    Backspace,
    /// Tab.
    Tab,
    /// Shift+Tab, enviado como `ESC [ Z`.
    BackTab,
    /// Escape.
    Escape,
    /// Seta para cima.
    Up,
    /// Seta para baixo.
    Down,
    /// Seta para a esquerda.
    Left,
    /// Seta para a direita.
    Right,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Insert.
    Insert,
    /// Delete.
    Delete,
    /// Tecla de função F1 a F12; outros números não enviam nada.
    F(u8),
    /// Shift+Page Up: rola a visão para trás; o terminal a consome, o programa não a vê.
    ScrollBack,
    /// Shift+Page Down: rola a visão para a frente; o programa não a vê.
    ScrollForward,
}

fn put(out: &mut [u8; 8], bytes: &[u8]) -> usize {
    out[..bytes.len()].copy_from_slice(bytes);
    bytes.len()
}

fn tilde(out: &mut [u8; 8], code: u8) -> usize {
    if code < 10 {
        put(out, &[0x1B, b'[', b'0' + code, b'~'])
    } else {
        put(out, &[0x1B, b'[', b'0' + code / 10, b'0' + code % 10, b'~'])
    }
}

fn cursor_key(out: &mut [u8; 8], application: bool, letter: u8) -> usize {
    let intro = if application { b'O' } else { b'[' };
    put(out, &[0x1B, intro, letter])
}

fn ctrl_code(c: char) -> Option<u8> {
    match u8::try_from(c).ok()? {
        b @ b'a'..=b'z' => Some(b - b'a' + 1),
        b @ b'A'..=b'Z' => Some(b - b'A' + 1),
        b'@' | b' ' => Some(0),
        b'[' => Some(0x1B),
        b'\\' => Some(0x1C),
        b']' => Some(0x1D),
        b'^' => Some(0x1E),
        b'_' => Some(0x1F),
        b'?' => Some(0x7F),
        _ => None,
    }
}

pub(crate) fn encode(key: Key, application: bool, out: &mut [u8; 8]) -> usize {
    match key {
        Key::Char(c) => c.encode_utf8(&mut out[..]).len(),
        Key::Ctrl(c) => ctrl_code(c).map_or(0, |code| put(out, &[code])),
        Key::Enter => put(out, b"\r"),
        Key::Backspace => put(out, &[0x7F]),
        Key::Tab => put(out, b"\t"),
        Key::BackTab => put(out, &[0x1B, b'[', b'Z']),
        Key::Escape => put(out, &[0x1B]),
        Key::Up => cursor_key(out, application, b'A'),
        Key::Down => cursor_key(out, application, b'B'),
        Key::Right => cursor_key(out, application, b'C'),
        Key::Left => cursor_key(out, application, b'D'),
        Key::Home => cursor_key(out, application, b'H'),
        Key::End => cursor_key(out, application, b'F'),
        Key::Insert => tilde(out, 2),
        Key::Delete => tilde(out, 3),
        Key::PageUp => tilde(out, 5),
        Key::PageDown => tilde(out, 6),
        Key::F(n @ 1..=4) => put(out, &[0x1B, b'O', b'P' + n - 1]),
        Key::F(n @ 5..=12) => tilde(out, [15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)]),
        Key::F(_) | Key::ScrollBack | Key::ScrollForward => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(key: Key, application: bool) -> Vec<u8> {
        let mut out = [0u8; 8];
        let n = encode(key, application, &mut out);
        out[..n].to_vec()
    }

    #[test]
    fn characters_are_sent_as_utf8() {
        assert_eq!(bytes(Key::Char('a'), false), b"a");
        assert_eq!(bytes(Key::Char('é'), false), "é".as_bytes());
        assert_eq!(bytes(Key::Char('€'), false), "€".as_bytes());
        assert_eq!(bytes(Key::Char('😀'), false), "😀".as_bytes());
    }

    #[test]
    fn editing_keys_send_the_xterm_bytes() {
        assert_eq!(bytes(Key::Enter, false), b"\r");
        assert_eq!(bytes(Key::Backspace, false), [0x7F]);
        assert_eq!(bytes(Key::Tab, false), b"\t");
        assert_eq!(bytes(Key::BackTab, false), b"\x1b[Z");
        assert_eq!(bytes(Key::Escape, false), [0x1B]);
        assert_eq!(bytes(Key::Insert, false), b"\x1b[2~");
        assert_eq!(bytes(Key::Delete, false), b"\x1b[3~");
        assert_eq!(bytes(Key::PageUp, false), b"\x1b[5~");
        assert_eq!(bytes(Key::PageDown, false), b"\x1b[6~");
        assert_eq!(bytes(Key::ScrollBack, false), b"");
        assert_eq!(bytes(Key::ScrollForward, true), b"");
    }

    #[test]
    fn cursor_keys_follow_the_application_mode() {
        let keys = [
            Key::Up,
            Key::Down,
            Key::Right,
            Key::Left,
            Key::Home,
            Key::End,
        ];
        let normal: [&[u8]; 6] = [
            b"\x1b[A", b"\x1b[B", b"\x1b[C", b"\x1b[D", b"\x1b[H", b"\x1b[F",
        ];
        let app: [&[u8]; 6] = [
            b"\x1bOA", b"\x1bOB", b"\x1bOC", b"\x1bOD", b"\x1bOH", b"\x1bOF",
        ];
        for (i, key) in keys.into_iter().enumerate() {
            assert_eq!(bytes(key, false), normal[i]);
            assert_eq!(bytes(key, true), app[i]);
        }
    }

    #[test]
    fn function_keys_cover_f1_to_f12() {
        assert_eq!(bytes(Key::F(1), false), b"\x1bOP");
        assert_eq!(bytes(Key::F(4), false), b"\x1bOS");
        assert_eq!(bytes(Key::F(5), false), b"\x1b[15~");
        assert_eq!(bytes(Key::F(6), false), b"\x1b[17~");
        assert_eq!(bytes(Key::F(10), false), b"\x1b[21~");
        assert_eq!(bytes(Key::F(11), false), b"\x1b[23~");
        assert_eq!(bytes(Key::F(12), false), b"\x1b[24~");
    }

    #[test]
    fn function_keys_out_of_range_send_nothing() {
        assert_eq!(bytes(Key::F(0), false).len(), 0);
        assert_eq!(bytes(Key::F(13), false).len(), 0);
        assert_eq!(bytes(Key::F(255), true).len(), 0);
    }

    #[test]
    fn ctrl_letters_map_to_control_codes() {
        assert_eq!(bytes(Key::Ctrl('a'), false), [1]);
        assert_eq!(bytes(Key::Ctrl('C'), false), [3]);
        assert_eq!(bytes(Key::Ctrl('z'), false), [26]);
        assert_eq!(bytes(Key::Ctrl('['), false), [0x1B]);
        assert_eq!(bytes(Key::Ctrl('\\'), false), [0x1C]);
        assert_eq!(bytes(Key::Ctrl(' '), false), [0]);
        assert_eq!(bytes(Key::Ctrl('?'), false), [0x7F]);
    }

    #[test]
    fn ctrl_of_an_unmapped_character_sends_nothing() {
        assert_eq!(bytes(Key::Ctrl('1'), false).len(), 0);
        assert_eq!(bytes(Key::Ctrl('é'), false).len(), 0);
    }
}
