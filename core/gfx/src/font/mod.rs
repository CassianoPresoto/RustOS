//! Fonte bitmap própria 8x16 do RustOS, com as tabelas montadas na compilação.
//! `glyphs.txt`: `U+XXXX [rótulo]` e 16 linhas de 8 colunas (`#` aceso, `.` apagado), em código
//! crescente; `;` abre comentário. Formato errado é panic em `const`, com a linha do erro.

mod parse;

use parse::{Entry, count, fail, parse};

pub const WIDTH: usize = 8;
pub const HEIGHT: usize = 16;

/// Uma célula: linha 0 no topo, bit 7 de cada byte é a coluna 0.
pub type Glyph = [u8; HEIGHT];

const SOURCE: &str = include_str!("glyphs.txt");
const REPLACEMENT_CODE: u16 = 0xFFFD;

const ASCII_FIRST: u32 = 0x20;
const ASCII_LAST: u32 = 0x7E;
const ASCII_LEN: usize = 95;
const HIGH_FIRST: u32 = 0xA0;
const HIGH_LAST: u32 = 0xFF;
const HIGH_LEN: usize = 96;
const DIRECT_LEN: usize = ASCII_LEN + HIGH_LEN;

const ENTRY_COUNT: usize = count(SOURCE);
const ENTRIES: [Entry; ENTRY_COUNT] = match parse(SOURCE) {
    Ok(entries) => entries,
    Err(error) => fail(error),
};
const EXTRA_LEN: usize = {
    assert!(
        ENTRY_COUNT >= DIRECT_LEN,
        "glyphs.txt não cobre U+0020..=U+007E e U+00A0..=U+00FF"
    );
    ENTRY_COUNT - DIRECT_LEN
};
const EXTRAS: ([u16; EXTRA_LEN], [Glyph; EXTRA_LEN]) = extras(&ENTRIES);
const REPLACEMENT_INDEX: usize = position(&EXTRAS.0, REPLACEMENT_CODE);

/// U+0020..=U+007E e U+00A0..=U+00FF, indexados por conta, sem vaga para C0/C1.
static DIRECT: [Glyph; DIRECT_LEN] = direct(&ENTRIES);
/// Códigos do resto, em ordem crescente; o glifo `i` mora em `EXTRA_GLYPHS[i]`.
static EXTRA_CODES: [u16; EXTRA_LEN] = EXTRAS.0;
static EXTRA_GLYPHS: [Glyph; EXTRA_LEN] = EXTRAS.1;

const fn direct_index(code: u32) -> Option<usize> {
    match code {
        ASCII_FIRST..=ASCII_LAST => Some((code - ASCII_FIRST) as usize),
        HIGH_FIRST..=HIGH_LAST => Some((code - HIGH_FIRST) as usize + ASCII_LEN),
        _ => None,
    }
}

const fn direct(entries: &[Entry]) -> [Glyph; DIRECT_LEN] {
    let mut table = [[0; HEIGHT]; DIRECT_LEN];
    let mut filled = 0;
    let mut index = 0;
    while index < entries.len() {
        if let Some(slot) = direct_index(entries[index].code as u32) {
            table[slot] = entries[index].glyph;
            filled += 1;
        }
        index += 1;
    }
    assert!(
        filled == DIRECT_LEN,
        "faltam glifos em U+0020..=U+007E ou U+00A0..=U+00FF"
    );
    table
}

const fn extras<const M: usize>(entries: &[Entry]) -> ([u16; M], [Glyph; M]) {
    let mut codes = [0u16; M];
    let mut glyphs = [[0; HEIGHT]; M];
    let mut next = 0;
    let mut index = 0;
    while index < entries.len() {
        let entry = entries[index];
        if direct_index(entry.code as u32).is_none() {
            assert!(
                next < M,
                "glyphs.txt tem mais glifos fora das faixas diretas"
            );
            codes[next] = entry.code;
            glyphs[next] = entry.glyph;
            next += 1;
        }
        index += 1;
    }
    (codes, glyphs)
}

const fn position(codes: &[u16], code: u16) -> usize {
    let mut index = 0;
    while index < codes.len() {
        if codes[index] == code {
            return index;
        }
        index += 1;
    }
    panic!("glyphs.txt precisa do glifo de substituição U+FFFD")
}

fn lookup(ch: char) -> Option<&'static Glyph> {
    let code = u32::from(ch);
    if let Some(slot) = direct_index(code) {
        return DIRECT.get(slot);
    }
    let code = u16::try_from(code).ok()?;
    let slot = EXTRA_CODES.binary_search(&code).ok()?;
    EXTRA_GLYPHS.get(slot)
}

/// Glifo de `ch`; sem glifo, o de U+FFFD. Busca O(1) em ASCII e Latin-1, binária no resto.
pub fn glyph(ch: char) -> &'static Glyph {
    lookup(ch).unwrap_or(&EXTRA_GLYPHS[REPLACEMENT_INDEX])
}

/// `true` se a fonte desenha `ch` (o substituto de `glyph` não conta).
pub fn has_glyph(ch: char) -> bool {
    lookup(ch).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_blank(glyph: &Glyph) -> bool {
        glyph.iter().all(|&row| row == 0)
    }

    fn rows_with_ink(glyph: &Glyph) -> Vec<usize> {
        (0..HEIGHT).filter(|&row| glyph[row] != 0).collect()
    }

    fn first_ink(ch: char) -> usize {
        rows_with_ink(glyph(ch))[0]
    }

    fn last_ink(ch: char) -> usize {
        *rows_with_ink(glyph(ch)).last().unwrap()
    }

    fn parsed() -> Vec<Entry> {
        match parse::<ENTRY_COUNT>(SOURCE) {
            Ok(entries) => entries.to_vec(),
            Err(error) => panic!("glyphs.txt:{}: {}", error.line, error.message),
        }
    }

    fn is_box_or_block(code: u16) -> bool {
        (0x2500..=0x259F).contains(&code)
    }

    #[test]
    fn bundled_glyph_file_parses_at_runtime_with_the_compile_time_count() {
        assert_eq!(parsed().len(), count(SOURCE));
        assert!(parsed().windows(2).all(|pair| pair[0].code < pair[1].code));
    }

    #[test]
    fn cell_is_eight_by_sixteen() {
        assert_eq!((WIDTH, HEIGHT), (8, 16));
        assert_eq!(size_of::<Glyph>(), 16);
    }

    #[test]
    fn space_and_no_break_space_are_blank() {
        assert!(is_blank(glyph(' ')));
        assert!(is_blank(glyph('\u{A0}')));
    }

    #[test]
    fn every_other_glyph_has_ink() {
        for entry in parsed() {
            if entry.code != 0x20 && entry.code != 0xA0 {
                assert!(
                    !is_blank(&entry.glyph),
                    "U+{:04X} está em branco",
                    entry.code
                );
            }
        }
    }

    #[test]
    fn ascii_and_latin1_are_fully_covered() {
        for code in (0x20..=0x7E).chain(0xA0..=0xFF) {
            let ch = char::from_u32(code).unwrap();
            assert!(has_glyph(ch), "falta U+{code:04X}");
        }
    }

    #[test]
    fn control_characters_have_no_glyph() {
        for code in (0x00..=0x1F).chain(0x7F..=0x9F) {
            assert!(!has_glyph(char::from_u32(code).unwrap()), "U+{code:04X}");
        }
    }

    #[test]
    fn required_symbols_are_covered() {
        let required = "─│┌┐└┘├┤┬┴┼╭╮╯╰█▀▄▌▐░▒▓■●▶◀▲▼←↑→↓•…–—‘’“”€✓✗⌘\u{FFFD}";
        for ch in required.chars() {
            assert!(has_glyph(ch), "falta {ch} (U+{:04X})", u32::from(ch));
        }
    }

    #[test]
    fn portuguese_letters_have_glyphs() {
        for ch in "áàâãéêíóôõúüçÁÀÂÃÉÊÍÓÔÕÚÜÇªº°".chars() {
            assert!(has_glyph(ch), "falta {ch}");
        }
    }

    #[test]
    fn lookup_matches_the_parsed_source_for_every_glyph() {
        for entry in parsed() {
            let ch = char::from_u32(u32::from(entry.code)).unwrap();
            assert_eq!(glyph(ch), &entry.glyph, "U+{:04X}", entry.code);
        }
    }

    #[test]
    fn unknown_characters_fall_back_to_the_replacement_glyph() {
        let replacement = glyph('\u{FFFD}');
        for ch in [
            '\0', '\u{1}', '\u{7F}', '\u{80}', '\u{9F}', '\u{100}', '\u{FFFF}', '😀',
        ] {
            assert!(!has_glyph(ch));
            assert_eq!(glyph(ch), replacement, "U+{:04X}", u32::from(ch));
        }
        assert!(has_glyph('\u{FFFD}'));
        assert_ne!(replacement, glyph('?'));
    }

    #[test]
    fn extra_table_is_sorted_without_duplicates() {
        assert!(EXTRA_CODES.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(
            EXTRA_CODES
                .iter()
                .all(|&code| direct_index(u32::from(code)).is_none())
        );
        assert_eq!(EXTRA_CODES[REPLACEMENT_INDEX], REPLACEMENT_CODE);
        assert_eq!(EXTRA_CODES.len() + DIRECT.len(), ENTRY_COUNT);
    }

    #[test]
    fn direct_index_skips_the_control_ranges() {
        assert_eq!(direct_index(0x1F), None);
        assert_eq!(direct_index(0x20), Some(0));
        assert_eq!(direct_index(0x7E), Some(94));
        assert_eq!(direct_index(0x7F), None);
        assert_eq!(direct_index(0x9F), None);
        assert_eq!(direct_index(0xA0), Some(95));
        assert_eq!(direct_index(0xFF), Some(190));
        assert_eq!(direct_index(0x100), None);
    }

    #[test]
    fn column_seven_is_empty_outside_boxes_and_blocks() {
        for entry in parsed() {
            if !is_box_or_block(entry.code) {
                for (row, bits) in entry.glyph.iter().enumerate() {
                    assert_eq!(bits & 1, 0, "U+{:04X} linha {row}", entry.code);
                }
            }
        }
    }

    #[test]
    fn capitals_and_digits_span_rows_three_to_twelve() {
        for ch in ('A'..='Z').chain('0'..='9') {
            assert_eq!((first_ink(ch), last_ink(ch)), (3, 12), "{ch}");
        }
    }

    #[test]
    fn lowercase_follows_x_height_ascender_and_descender_rows() {
        for ch in "acemnorsuvwxz".chars() {
            assert_eq!((first_ink(ch), last_ink(ch)), (6, 12), "{ch}");
        }
        for ch in "bdfhklt".chars() {
            assert_eq!((first_ink(ch), last_ink(ch)), (3, 12), "{ch}");
        }
        for ch in "gjpqy".chars() {
            assert_eq!(last_ink(ch), 15, "{ch}");
        }
        assert_eq!(last_ink('i'), 12);
    }

    #[test]
    fn lowercase_accents_use_rows_three_and_four_leaving_five_free() {
        for ch in "áàâãäåéèêëíìîïñóòôõöúùûüýÿ".chars() {
            let glyph = glyph(ch);
            assert!(
                glyph[..3].iter().all(|&row| row == 0),
                "{ch} acima da linha 3"
            );
            assert!(
                glyph[3] != 0 && glyph[4] != 0,
                "{ch} sem acento nas linhas 3-4"
            );
            assert_eq!(glyph[5], 0, "{ch} linha 5 ocupada");
        }
    }

    #[test]
    fn capital_accents_use_rows_zero_and_one_leaving_two_free() {
        for ch in "ÀÁÂÃÄÅÈÉÊËÌÍÎÏÑÒÓÔÕÖÙÚÛÜÝ".chars() {
            let glyph = glyph(ch);
            assert!(
                glyph[0] != 0 && glyph[1] != 0,
                "{ch} sem acento nas linhas 0-1"
            );
            assert_eq!(glyph[2], 0, "{ch} linha 2 ocupada");
            assert_eq!(last_ink(ch), 12, "{ch}");
        }
    }

    #[test]
    fn cedilla_hangs_in_rows_thirteen_and_fourteen() {
        for ch in ['ç', 'Ç'] {
            let glyph = glyph(ch);
            assert!(glyph[13] != 0 && glyph[14] != 0, "{ch}");
            assert_eq!(glyph[15], 0, "{ch}");
            assert_eq!(glyph[12] & 0xFE, glyph[12], "{ch}");
        }
    }

    #[test]
    fn thin_box_lines_use_row_eight_and_column_three() {
        assert_eq!(glyph('─')[8], 0xFF);
        assert_eq!(rows_with_ink(glyph('─')), [8]);
        assert!(glyph('│').iter().all(|&row| row == 0x10));
        for ch in ['┼', '├', '┤', '┬', '┴', '┌', '┐', '└', '┘'] {
            assert_eq!(glyph(ch)[8] & 0x10, 0x10, "{ch}");
        }
    }

    #[test]
    fn box_pieces_reach_the_cell_edges_to_join_neighbours() {
        let horizontal = |ch: char| glyph(ch)[8];
        assert_eq!(horizontal('┬'), 0xFF);
        assert_eq!(horizontal('┴'), 0xFF);
        assert_eq!(horizontal('┼'), 0xFF);
        assert_eq!(glyph('├')[0] & 0x10, 0x10);
        assert_eq!(glyph('┤')[15] & 0x10, 0x10);
        assert_eq!(glyph('┬')[15] & 0x10, 0x10);
        assert_eq!(glyph('┴')[0] & 0x10, 0x10);
    }

    #[test]
    fn rounded_corners_open_toward_the_expected_sides() {
        let left_edge = |ch: char| glyph(ch).iter().map(|row| row & 0x80).any(|bit| bit != 0);
        let right_edge = |ch: char| glyph(ch).iter().map(|row| row & 0x01).any(|bit| bit != 0);
        assert!(right_edge('╭') && !left_edge('╭') && glyph('╭')[15] == 0x10);
        assert!(left_edge('╮') && !right_edge('╮') && glyph('╮')[15] == 0x10);
        assert!(left_edge('╯') && !right_edge('╯') && glyph('╯')[0] == 0x10);
        assert!(right_edge('╰') && !left_edge('╰') && glyph('╰')[0] == 0x10);
    }

    #[test]
    fn block_elements_fill_their_halves() {
        assert!(glyph('█').iter().all(|&row| row == 0xFF));
        assert!(glyph('▀')[..8].iter().all(|&row| row == 0xFF));
        assert!(glyph('▀')[8..].iter().all(|&row| row == 0));
        assert!(glyph('▄')[8..].iter().all(|&row| row == 0xFF));
        assert!(glyph('▄')[..8].iter().all(|&row| row == 0));
        assert!(glyph('▌').iter().all(|&row| row == 0xF0));
        assert!(glyph('▐').iter().all(|&row| row == 0x0F));
    }

    #[test]
    fn shade_blocks_grow_denser() {
        let ink = |ch: char| glyph(ch).iter().map(|row| row.count_ones()).sum::<u32>();
        assert_eq!(ink('░'), 32);
        assert_eq!(ink('▒'), 64);
        assert_eq!(ink('▓'), 96);
    }

    #[test]
    fn look_alike_characters_are_distinguishable() {
        let groups = [
            "l1I|",
            "0OoQ",
            "5S",
            "2Z",
            "8B",
            "rn",
            "vV",
            ",.;:",
            "`'\"´‘’",
        ];
        for group in groups {
            let chars: Vec<char> = group.chars().collect();
            for (index, &a) in chars.iter().enumerate() {
                for &b in &chars[index + 1..] {
                    assert_ne!(glyph(a), glyph(b), "{a} e {b} saíram iguais");
                }
            }
        }
    }

    #[test]
    fn ascii_glyphs_are_pairwise_distinct() {
        for a in ' '..='~' {
            for b in (a..='~').skip(1) {
                assert_ne!(glyph(a), glyph(b), "{a:?} e {b:?} saíram iguais");
            }
        }
    }

    #[test]
    fn zero_is_marked_apart_from_capital_o() {
        let ink = |ch: char| glyph(ch).iter().map(|row| row.count_ones()).sum::<u32>();
        assert!(ink('0') > ink('O'));
    }

    #[test]
    fn tables_take_the_expected_rodata() {
        let direct = size_of_val(&DIRECT);
        let extra = size_of_val(&EXTRA_CODES) + size_of_val(&EXTRA_GLYPHS);
        assert_eq!(direct, DIRECT_LEN * HEIGHT);
        assert_eq!(extra, EXTRA_LEN * (HEIGHT + 2));
        assert!(direct + extra < 6 * 1024);
    }
}
