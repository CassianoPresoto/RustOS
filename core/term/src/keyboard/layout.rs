//! Mapas de tecla por layout: HID -> quatro níveis (base, Shift, AltGr,
//! AltGr+Shift). Tabela fixa e ordenada por HID, na ROM; sem alloc.

pub(super) const NONE: char = '\0';
/// Teclas mortas: pontos de código de uso privado, nunca digitáveis.
pub(super) const DEAD_ACUTE: char = '\u{E001}';
pub(super) const DEAD_GRAVE: char = '\u{E002}';
pub(super) const DEAD_TILDE: char = '\u{E003}';
pub(super) const DEAD_CIRCUMFLEX: char = '\u{E004}';
pub(super) const DEAD_DIAERESIS: char = '\u{E005}';

pub(super) type Row = (u8, [char; 4]);

/// `us` do xkeyboard-config.
pub(super) static US: [Row; 49] = [
    (0x04, ['a', 'A', NONE, NONE]),
    (0x05, ['b', 'B', NONE, NONE]),
    (0x06, ['c', 'C', NONE, NONE]),
    (0x07, ['d', 'D', NONE, NONE]),
    (0x08, ['e', 'E', NONE, NONE]),
    (0x09, ['f', 'F', NONE, NONE]),
    (0x0A, ['g', 'G', NONE, NONE]),
    (0x0B, ['h', 'H', NONE, NONE]),
    (0x0C, ['i', 'I', NONE, NONE]),
    (0x0D, ['j', 'J', NONE, NONE]),
    (0x0E, ['k', 'K', NONE, NONE]),
    (0x0F, ['l', 'L', NONE, NONE]),
    (0x10, ['m', 'M', NONE, NONE]),
    (0x11, ['n', 'N', NONE, NONE]),
    (0x12, ['o', 'O', NONE, NONE]),
    (0x13, ['p', 'P', NONE, NONE]),
    (0x14, ['q', 'Q', NONE, NONE]),
    (0x15, ['r', 'R', NONE, NONE]),
    (0x16, ['s', 'S', NONE, NONE]),
    (0x17, ['t', 'T', NONE, NONE]),
    (0x18, ['u', 'U', NONE, NONE]),
    (0x19, ['v', 'V', NONE, NONE]),
    (0x1A, ['w', 'W', NONE, NONE]),
    (0x1B, ['x', 'X', NONE, NONE]),
    (0x1C, ['y', 'Y', NONE, NONE]),
    (0x1D, ['z', 'Z', NONE, NONE]),
    (0x1E, ['1', '!', NONE, NONE]),
    (0x1F, ['2', '@', NONE, NONE]),
    (0x20, ['3', '#', NONE, NONE]),
    (0x21, ['4', '$', NONE, NONE]),
    (0x22, ['5', '%', NONE, NONE]),
    (0x23, ['6', '^', NONE, NONE]),
    (0x24, ['7', '&', NONE, NONE]),
    (0x25, ['8', '*', NONE, NONE]),
    (0x26, ['9', '(', NONE, NONE]),
    (0x27, ['0', ')', NONE, NONE]),
    (0x2C, [' ', ' ', NONE, NONE]),
    (0x2D, ['-', '_', NONE, NONE]),
    (0x2E, ['=', '+', NONE, NONE]),
    (0x2F, ['[', '{', NONE, NONE]),
    (0x30, [']', '}', NONE, NONE]),
    (0x31, ['\\', '|', NONE, NONE]),
    (0x33, [';', ':', NONE, NONE]),
    (0x34, ['\'', '"', NONE, NONE]),
    (0x35, ['`', '~', NONE, NONE]),
    (0x36, [',', '<', NONE, NONE]),
    (0x37, ['.', '>', NONE, NONE]),
    (0x38, ['/', '?', NONE, NONE]),
    (0x64, ['<', '>', NONE, NONE]),
];

/// `br(abnt2)` do xkeyboard-config, só os símbolos que existem em Latin-1.
pub(super) static ABNT2: [Row; 51] = [
    (0x04, ['a', 'A', 'æ', 'Æ']),
    (0x05, ['b', 'B', NONE, NONE]),
    (0x06, ['c', 'C', '©', '©']),
    (0x07, ['d', 'D', 'ð', 'Ð']),
    (0x08, ['e', 'E', '°', '°']),
    (0x09, ['f', 'F', NONE, 'ª']),
    (0x0A, ['g', 'G', NONE, NONE]),
    (0x0B, ['h', 'H', NONE, NONE]),
    (0x0C, ['i', 'I', NONE, NONE]),
    (0x0D, ['j', 'J', NONE, NONE]),
    (0x0E, ['k', 'K', NONE, '&']),
    (0x0F, ['l', 'L', NONE, NONE]),
    (0x10, ['m', 'M', 'µ', 'º']),
    (0x11, ['n', 'N', NONE, NONE]),
    (0x12, ['o', 'O', 'ø', 'Ø']),
    (0x13, ['p', 'P', 'þ', 'Þ']),
    (0x14, ['q', 'Q', '/', '/']),
    (0x15, ['r', 'R', '®', '®']),
    (0x16, ['s', 'S', 'ß', NONE]),
    (0x17, ['t', 'T', NONE, NONE]),
    (0x18, ['u', 'U', NONE, NONE]),
    (0x19, ['v', 'V', NONE, NONE]),
    (0x1A, ['w', 'W', '?', '?']),
    (0x1B, ['x', 'X', '»', '>']),
    (0x1C, ['y', 'Y', NONE, '¥']),
    (0x1D, ['z', 'Z', '«', '<']),
    (0x1E, ['1', '!', '¹', '¡']),
    (0x1F, ['2', '@', '²', '½']),
    (0x20, ['3', '#', '³', '¾']),
    (0x21, ['4', '$', '£', '¼']),
    (0x22, ['5', '%', '¢', NONE]),
    (0x23, ['6', DEAD_DIAERESIS, '¬', '¨']),
    (0x24, ['7', '&', '{', NONE]),
    (0x25, ['8', '*', '[', NONE]),
    (0x26, ['9', '(', ']', '±']),
    (0x27, ['0', ')', '}', '°']),
    (0x2C, [' ', ' ', NONE, NONE]),
    (0x2D, ['-', '_', '\\', '¿']),
    (0x2E, ['=', '+', '§', NONE]),
    (0x2F, [DEAD_ACUTE, DEAD_GRAVE, '´', '`']),
    (0x30, ['[', '{', 'ª', NONE]),
    (0x31, [']', '}', 'º', 'º']),
    (0x32, [']', '}', 'º', 'º']),
    (0x33, ['ç', 'Ç', DEAD_ACUTE, NONE]),
    (0x34, [DEAD_TILDE, DEAD_CIRCUMFLEX, '~', '^']),
    (0x35, ['\'', '"', '¬', '¬']),
    (0x36, [',', '<', NONE, '×']),
    (0x37, ['.', '>', '·', '÷']),
    (0x38, [';', ':', NONE, NONE]),
    (0x64, ['\\', '|', NONE, NONE]),
    (0x87, ['/', '?', '°', '¿']),
];

pub(super) fn find(table: &[Row], code: u8) -> Option<[char; 4]> {
    table
        .binary_search_by_key(&code, |&(hid, _)| hid)
        .ok()
        .map(|i| table[i].1)
}

pub(super) fn is_dead(c: char) -> bool {
    matches!(
        c,
        DEAD_ACUTE | DEAD_GRAVE | DEAD_TILDE | DEAD_CIRCUMFLEX | DEAD_DIAERESIS
    )
}

/// O acento que a tecla morta escreve sozinha (seguida de espaço).
pub(super) fn accent(dead: char) -> char {
    match dead {
        DEAD_ACUTE => '\u{B4}',
        DEAD_GRAVE => '`',
        DEAD_TILDE => '~',
        DEAD_CIRCUMFLEX => '^',
        _ => '\u{A8}',
    }
}

const ACUTE: &[(char, char)] = &[
    ('a', 'á'),
    ('e', 'é'),
    ('i', 'í'),
    ('o', 'ó'),
    ('u', 'ú'),
    ('y', 'ý'),
    ('c', 'ç'),
    ('A', 'Á'),
    ('E', 'É'),
    ('I', 'Í'),
    ('O', 'Ó'),
    ('U', 'Ú'),
    ('Y', 'Ý'),
    ('C', 'Ç'),
];
const GRAVE: &[(char, char)] = &[
    ('a', 'à'),
    ('e', 'è'),
    ('i', 'ì'),
    ('o', 'ò'),
    ('u', 'ù'),
    ('A', 'À'),
    ('E', 'È'),
    ('I', 'Ì'),
    ('O', 'Ò'),
    ('U', 'Ù'),
];
const TILDE: &[(char, char)] = &[
    ('a', 'ã'),
    ('o', 'õ'),
    ('n', 'ñ'),
    ('A', 'Ã'),
    ('O', 'Õ'),
    ('N', 'Ñ'),
];
const CIRCUMFLEX: &[(char, char)] = &[
    ('a', 'â'),
    ('e', 'ê'),
    ('i', 'î'),
    ('o', 'ô'),
    ('u', 'û'),
    ('A', 'Â'),
    ('E', 'Ê'),
    ('I', 'Î'),
    ('O', 'Ô'),
    ('U', 'Û'),
];
const DIAERESIS: &[(char, char)] = &[
    ('a', 'ä'),
    ('e', 'ë'),
    ('i', 'ï'),
    ('o', 'ö'),
    ('u', 'ü'),
    ('y', 'ÿ'),
    ('A', 'Ä'),
    ('E', 'Ë'),
    ('I', 'Ï'),
    ('O', 'Ö'),
    ('U', 'Ü'),
];

/// Letra acentuada pela tecla morta, quando existe em Latin-1.
pub(super) fn compose(dead: char, base: char) -> Option<char> {
    let pairs = match dead {
        DEAD_ACUTE => ACUTE,
        DEAD_GRAVE => GRAVE,
        DEAD_TILDE => TILDE,
        DEAD_CIRCUMFLEX => CIRCUMFLEX,
        _ => DIAERESIS,
    };
    pairs.iter().find(|&&(b, _)| b == base).map(|&(_, c)| c)
}
