//! Parser do arquivo de glifos: roda na compilação (`const fn`) e nos testes, com o mesmo código.

use super::{Glyph, HEIGHT, WIDTH};

/// Primeiro defeito achado no arquivo, com a linha (a partir de 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: &'static str,
}

/// Um glifo lido: código Unicode do plano básico e as linhas do bitmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub code: u16,
    pub glyph: Glyph,
}

pub const EMPTY: Entry = Entry {
    code: 0,
    glyph: [0; HEIGHT],
};

const ON: u8 = b'#';
const OFF: u8 = b'.';
const COMMENT: u8 = b';';

struct Line {
    start: usize,
    end: usize,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
    number: usize,
}

impl<'a> Cursor<'a> {
    const fn new(source: &'a str) -> Self {
        Self {
            bytes: source.as_bytes(),
            pos: 0,
            number: 0,
        }
    }

    const fn next_line(&mut self) -> Option<Line> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let start = self.pos;
        let mut end = start;
        while end < self.bytes.len() && self.bytes[end] != b'\n' {
            end += 1;
        }
        self.pos = end + 1;
        self.number += 1;
        if end > start && self.bytes[end - 1] == b'\r' {
            end -= 1;
        }
        Some(Line { start, end })
    }

    const fn is_header(&self, line: &Line) -> bool {
        line.end - line.start >= 2
            && self.bytes[line.start] == b'U'
            && self.bytes[line.start + 1] == b'+'
    }

    const fn is_blank(&self, line: &Line) -> bool {
        let mut index = line.start;
        while index < line.end {
            if !self.bytes[index].is_ascii_whitespace() {
                return false;
            }
            index += 1;
        }
        true
    }

    const fn is_comment(&self, line: &Line) -> bool {
        line.end > line.start && self.bytes[line.start] == COMMENT
    }

    const fn error(&self, message: &'static str) -> ParseError {
        ParseError {
            line: self.number,
            message,
        }
    }

    /// `U+XXXX` (4 a 6 dígitos hex), opcionalmente seguido de espaço e rótulo livre.
    const fn header_code(&self, line: &Line) -> Result<u32, ParseError> {
        let mut index = line.start + 2;
        let digits_start = index;
        let mut code = 0u32;
        while index < line.end && index - digits_start < 6 {
            let Some(digit) = hex_digit(self.bytes[index]) else {
                break;
            };
            code = code * 16 + digit;
            index += 1;
        }
        let digits = index - digits_start;
        if digits < 4 {
            return Err(self.error("código deve ter de 4 a 6 dígitos hexadecimais depois de U+"));
        }
        if index < line.end && !self.bytes[index].is_ascii_whitespace() {
            return Err(self.error("depois do código só cabe espaço e um rótulo"));
        }
        Ok(code)
    }

    const fn row(&self, line: &Line) -> Result<u8, ParseError> {
        if line.end - line.start != WIDTH {
            return Err(self.error("linha do glifo precisa ter exatamente 8 colunas"));
        }
        let mut bits = 0u8;
        let mut column = 0;
        while column < WIDTH {
            bits <<= 1;
            match self.bytes[line.start + column] {
                ON => bits |= 1,
                OFF => {}
                _ => return Err(self.error("pixel deve ser `#` (aceso) ou `.` (apagado)")),
            }
            column += 1;
        }
        Ok(bits)
    }
}

const fn hex_digit(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some((byte - b'0') as u32),
        b'a'..=b'f' => Some((byte - b'a') as u32 + 10),
        b'A'..=b'F' => Some((byte - b'A') as u32 + 10),
        _ => None,
    }
}

/// Quantos glifos o texto declara (linhas que começam com `U+`).
pub const fn count(source: &str) -> usize {
    let mut cursor = Cursor::new(source);
    let mut total = 0;
    while let Some(line) = cursor.next_line() {
        if cursor.is_header(&line) {
            total += 1;
        }
    }
    total
}

/// Lê os `N` glifos do texto, em ordem estrita de código.
#[allow(
    clippy::cast_possible_truncation,
    reason = "o código já foi limitado a U+FFFF"
)]
pub const fn parse<const N: usize>(source: &str) -> Result<[Entry; N], ParseError> {
    let mut cursor = Cursor::new(source);
    let mut entries = [EMPTY; N];
    let mut found = 0;
    while let Some(line) = cursor.next_line() {
        if cursor.is_blank(&line) || cursor.is_comment(&line) {
            continue;
        }
        if !cursor.is_header(&line) {
            return Err(cursor.error("esperava `U+XXXX`, comentário `;` ou linha em branco"));
        }
        let code = match cursor.header_code(&line) {
            Ok(code) => code,
            Err(error) => return Err(error),
        };
        if code > 0xFFFF {
            return Err(cursor.error("só o plano básico (até U+FFFF) tem glifo"));
        }
        if char::from_u32(code).is_none() {
            return Err(cursor.error("código não é um caractere Unicode válido"));
        }
        if found > 0 && code == entries[found - 1].code as u32 {
            return Err(cursor.error("código duplicado"));
        }
        if found > 0 && code < entries[found - 1].code as u32 {
            return Err(cursor.error("código fora de ordem: o arquivo é crescente"));
        }
        if found == N {
            return Err(cursor.error("mais glifos do que o contado"));
        }
        let mut glyph: Glyph = [0; HEIGHT];
        let mut row = 0;
        while row < HEIGHT {
            let Some(line) = cursor.next_line() else {
                return Err(cursor.error("o arquivo acaba antes da 16ª linha do glifo"));
            };
            glyph[row] = match cursor.row(&line) {
                Ok(bits) => bits,
                Err(error) => return Err(error),
            };
            row += 1;
        }
        entries[found] = Entry {
            code: code as u16,
            glyph,
        };
        found += 1;
    }
    if found != N {
        return Err(cursor.error("menos glifos do que o contado"));
    }
    Ok(entries)
}

/// Aborta a compilação com `glyphs.txt:LINHA: mensagem`.
#[allow(clippy::cast_possible_truncation, reason = "`% 10` cabe em u8")]
pub const fn fail(error: ParseError) -> ! {
    const PREFIX: &[u8] = b"font/glyphs.txt:";
    let mut buffer = [0u8; 160];
    let mut len = 0;
    while len < PREFIX.len() {
        buffer[len] = PREFIX[len];
        len += 1;
    }
    let mut divisor = 1;
    while error.line / divisor >= 10 {
        divisor *= 10;
    }
    while divisor > 0 {
        buffer[len] = b'0' + (error.line / divisor % 10) as u8;
        len += 1;
        divisor /= 10;
    }
    let message = error.message.as_bytes();
    buffer[len] = b':';
    buffer[len + 1] = b' ';
    len += 2;
    let mut index = 0;
    while index < message.len() && len < buffer.len() {
        buffer[len] = message[index];
        len += 1;
        index += 1;
    }
    let (text, _) = buffer.split_at(len);
    match core::str::from_utf8(text) {
        Ok(text) => panic!("{}", text),
        Err(_) => panic!("font/glyphs.txt malformado"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLANK_ROWS: &str = "........\n........\n........\n........\n........\n........\n\
                              ........\n........\n........\n........\n........\n........\n\
                              ........\n........\n........\n........\n";

    fn glyph_text(header: &str) -> String {
        format!("{header}\n{BLANK_ROWS}")
    }

    fn parse_one(text: &str) -> Result<[Entry; 1], ParseError> {
        parse::<1>(text)
    }

    #[test]
    fn counts_only_header_lines() {
        let text = format!(
            "; nota\n\n{}{}",
            glyph_text("U+0041"),
            glyph_text("U+0042 B")
        );
        assert_eq!(count(&text), 2);
        assert_eq!(count(""), 0);
        assert_eq!(count("; só comentário\n"), 0);
    }

    #[test]
    fn parses_bits_with_column_zero_as_most_significant() {
        let mut text = String::from("U+0041\n#.......\n.......#\n##......\n");
        text.push_str(&BLANK_ROWS.lines().skip(3).collect::<Vec<_>>().join("\n"));
        text.push('\n');
        let [entry] = parse_one(&text).unwrap();
        assert_eq!(entry.code, 0x41);
        assert_eq!(&entry.glyph[..3], &[0x80, 0x01, 0xC0]);
        assert!(entry.glyph[3..].iter().all(|&row| row == 0));
    }

    #[test]
    fn accepts_comments_blank_lines_labels_and_crlf() {
        let text = format!(
            "; início\r\n\r\n{}\n; entre glifos\n{}",
            glyph_text("U+0042 B maiúsculo"),
            glyph_text("U+00e9 é").replace('\n', "\r\n")
        );
        let entries = parse::<2>(&text).unwrap();
        assert_eq!(entries[0].code, 0x42);
        assert_eq!(entries[1].code, 0xE9);
    }

    #[test]
    fn rejects_codes_beyond_the_basic_plane() {
        let error = parse_one(&glyph_text("U+010000")).unwrap_err();
        assert_eq!(error.message, "só o plano básico (até U+FFFF) tem glifo");
    }

    #[test]
    fn rejects_rows_with_wrong_width_and_reports_the_line() {
        let text = glyph_text("U+0041").replacen("........", ".......", 3);
        let error = parse_one(&text).unwrap_err();
        assert_eq!(error.line, 2);
        assert!(error.message.contains("8 colunas"));
        let wide = glyph_text("U+0041").replacen("........", "#########", 1);
        assert_eq!(parse_one(&wide).unwrap_err().line, 2);
    }

    #[test]
    fn rejects_unknown_pixel_characters() {
        let text = glyph_text("U+0041").replacen("........", "..X.....", 1);
        let error = parse_one(&text).unwrap_err();
        assert_eq!((error.line, error.message.contains("`#`")), (2, true));
    }

    #[test]
    fn rejects_glyphs_with_too_few_or_too_many_rows() {
        let short = glyph_text("U+0041").replacen("........\n", "", 1);
        let error = parse_one(&short).unwrap_err();
        assert!(error.message.contains("16ª linha"), "{error:?}");
        let long = format!("{}........\n", glyph_text("U+0041"));
        let error = parse_one(&long).unwrap_err();
        assert_eq!(error.line, 18);
        assert!(error.message.contains("esperava"));
    }

    #[test]
    fn rejects_missing_row_when_the_next_glyph_starts_early() {
        let text = format!(
            "{}{}",
            glyph_text("U+0041").replacen("........\n", "", 1),
            glyph_text("U+0042")
        );
        let error = parse::<2>(&text).unwrap_err();
        assert_eq!(error.line, 17);
    }

    #[test]
    fn rejects_duplicate_and_out_of_order_codes() {
        let duplicate = format!("{}{}", glyph_text("U+0041"), glyph_text("U+0041"));
        let error = parse::<2>(&duplicate).unwrap_err();
        assert_eq!((error.line, error.message), (18, "código duplicado"));
        let reversed = format!("{}{}", glyph_text("U+0042"), glyph_text("U+0041"));
        let error = parse::<2>(&reversed).unwrap_err();
        assert_eq!(error.line, 18);
        assert!(error.message.contains("fora de ordem"));
    }

    #[test]
    fn rejects_malformed_headers() {
        for header in ["U+41", "U+00G1", "U+0041x", "U+", "U+D800", "U+0041;"] {
            assert!(parse_one(&glyph_text(header)).is_err(), "{header}");
        }
    }

    #[test]
    fn rejects_text_that_is_neither_header_comment_nor_blank() {
        let error = parse_one("lixo\n").unwrap_err();
        assert_eq!(error.line, 1);
    }

    #[test]
    fn rejects_a_count_that_disagrees_with_the_text() {
        let two = format!("{}{}", glyph_text("U+0041"), glyph_text("U+0042"));
        assert!(parse::<1>(&two).is_err());
        assert!(parse::<3>(&two).is_err());
    }

    #[test]
    fn fail_renders_line_and_message_in_the_panic() {
        let caught = std::panic::catch_unwind(|| {
            fail(ParseError {
                line: 1203,
                message: "código duplicado",
            })
        })
        .unwrap_err();
        let text = caught.downcast_ref::<String>().cloned().unwrap_or_else(|| {
            caught
                .downcast_ref::<&str>()
                .map_or_default(ToString::to_string)
        });
        assert!(text.contains("glyphs.txt:1203: código duplicado"), "{text}");
    }

    #[test]
    fn empty_entry_is_blank() {
        assert_eq!(EMPTY.code, 0);
        assert!(EMPTY.glyph.iter().all(|&row| row == 0));
    }
}
