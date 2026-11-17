//! `font-preview`: desenha a grade de glifos e amostras de texto num PNG, só com `std`.
//! Lê `core/gfx/src/font/glyphs.txt` com parser próprio: o xtask não depende do workspace.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::png;

const GLYPHS_PATH: &str = "core/gfx/src/font/glyphs.txt";
const OUTPUT_PATH: &str = "target/font-preview.png";
const WIDTH: usize = 8;
const HEIGHT: usize = 16;
const REPLACEMENT: u32 = 0xFFFD;
const DEFAULT_SCALE: usize = 2;
const MAX_SCALE: usize = 8;
const GRID_COLUMNS: usize = 32;
const MARGIN: usize = 16;
const LABEL_WIDTH: usize = 4 * WIDTH;

const PALETTE: [[u8; 3]; 6] = [
    [0x1E, 0x1E, 0x1E],
    [0xD4, 0xD4, 0xD4],
    [0x80, 0x80, 0x80],
    [0x27, 0x27, 0x27],
    [0x33, 0x33, 0x3C],
    [0x56, 0x9C, 0xD6],
];
const BACKGROUND: u8 = 0;
const FOREGROUND: u8 = 1;
const LABEL: u8 = 2;
const CELL: u8 = 3;
const GUIDE: u8 = 4;
const HEADING: u8 = 5;

const ASCII_SAMPLE: &str =
    "The quick brown fox jumps over the lazy dog 0123456789 {}[]()<>;:'\"`~!@#$%^&*";
const PANGRAM: &str = "À noite, vovô Kowalsky vê o ímã cair no pé do pinguim queixoso e vovó põe açúcar no chá de tâmaras do jabuti feliz.";
const BOX_LINES: [&str; 4] = [
    "RustOS 0.1  ─  terminal em português",
    "$ ls ~/açúcar   ✓ ok   ✗ falhou   ⌘ €",
    "● ▶ ◀ ▲ ▼  ← ↑ → ↓  • … – — ‘a’ “b”",
    "█ ▀ ▄ ▌ ▐ ░ ▒ ▓  ■  ½ ¼ ¾ ª º ° ± × ÷",
];
const TABLE: [&str; 5] = [
    "┌────────┬────────┐",
    "│  Olá   │ Mundo  │",
    "├────────┼────────┤",
    "│ 1  ✓   │ 2  ✗   │",
    "└────────┴────────┘",
];

type Glyph = [u8; HEIGHT];

/// Comando `font-preview [--scale N]`.
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let scale = parse_scale(args)?;
    let source = fs::read_to_string(root.join(GLYPHS_PATH))
        .map_err(|error| format!("não consegui ler {GLYPHS_PATH}: {error}"))?;
    let font = Font::parse(&source)?;
    let output = root.join(OUTPUT_PATH);
    let canvas = write_preview(&font, scale, &output)?;
    println!(
        "font-preview: {} glifos, {}x{} px, escala {scale} -> {OUTPUT_PATH}",
        font.len(),
        canvas.width,
        canvas.height
    );
    Ok(())
}

fn write_preview(font: &Font, scale: usize, output: &Path) -> Result<Canvas, String> {
    let canvas = render(font, scale);
    let png = png::encode_indexed(canvas.width, canvas.height, &PALETTE, &canvas.pixels)?;
    if let Some(dir) = output.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| format!("não consegui criar {}: {error}", dir.display()))?;
    }
    fs::write(output, &png)
        .map_err(|error| format!("não consegui gravar {}: {error}", output.display()))?;
    Ok(canvas)
}

fn parse_scale(args: &[String]) -> Result<usize, String> {
    let mut scale = DEFAULT_SCALE;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        if flag != "--scale" {
            return Err(format!(
                "opção desconhecida `{flag}` (uso: font-preview [--scale N])"
            ));
        }
        let value = args
            .next()
            .ok_or_else(|| "`--scale` precisa de valor".to_owned())?;
        scale = value
            .parse::<usize>()
            .ok()
            .filter(|scale| (1..=MAX_SCALE).contains(scale))
            .ok_or_else(|| format!("`--scale {value}`: use um inteiro de 1 a {MAX_SCALE}"))?;
    }
    Ok(scale)
}

/// Glifos por código, lidos do mesmo arquivo de texto que o kernel compila.
#[derive(Debug)]
struct Font {
    glyphs: BTreeMap<u32, Glyph>,
}

impl Font {
    fn parse(text: &str) -> Result<Self, String> {
        let mut glyphs = BTreeMap::new();
        let mut lines = text.lines().enumerate();
        while let Some((index, line)) = lines.next() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            let at = |message: &str| format!("glyphs.txt:{}: {message}", index + 1);
            let code = parse_code(line).ok_or_else(|| at("esperava `U+XXXX`"))?;
            let mut glyph = [0u8; HEIGHT];
            for row in &mut glyph {
                let (index, text) = lines
                    .next()
                    .ok_or_else(|| at("o arquivo acaba no meio de um glifo"))?;
                *row = parse_row(text).ok_or_else(|| {
                    format!(
                        "glyphs.txt:{}: linha deve ter 8 colunas de `#` e `.`",
                        index + 1
                    )
                })?;
            }
            if glyphs.insert(code, glyph).is_some() {
                return Err(at("código duplicado"));
            }
        }
        if glyphs.contains_key(&REPLACEMENT) {
            Ok(Self { glyphs })
        } else {
            Err("glyphs.txt: falta o glifo de substituição U+FFFD".to_owned())
        }
    }

    fn len(&self) -> usize {
        self.glyphs.len()
    }

    /// Glifo de `ch`, ou o de U+FFFD.
    fn glyph(&self, ch: char) -> &Glyph {
        let fallback = &self.glyphs[&REPLACEMENT];
        self.glyphs.get(&u32::from(ch)).unwrap_or(fallback)
    }
}

fn parse_code(line: &str) -> Option<u32> {
    let rest = line.strip_prefix("U+")?;
    let digits = rest.split_whitespace().next()?;
    u32::from_str_radix(digits, 16).ok()
}

fn parse_row(text: &str) -> Option<u8> {
    if text.len() != WIDTH {
        return None;
    }
    text.bytes().try_fold(0u8, |bits, byte| match byte {
        b'#' => Some(bits << 1 | 1),
        b'.' => Some(bits << 1),
        _ => None,
    })
}

/// Imagem de índices de paleta que cresce para baixo.
struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl Canvas {
    fn new(width: usize) -> Self {
        Self {
            width,
            height: 0,
            pixels: Vec::new(),
        }
    }

    fn grow_to(&mut self, height: usize) {
        if height > self.height {
            self.pixels.resize(self.width * height, BACKGROUND);
            self.height = height;
        }
    }

    fn fill_rect(&mut self, x: usize, y: usize, width: usize, height: usize, color: u8) {
        self.grow_to(y + height);
        for row in y..y + height {
            let start = row * self.width + x.min(self.width);
            let end = row * self.width + (x + width).min(self.width);
            self.pixels[start..end].fill(color);
        }
    }

    fn draw_glyph(&mut self, x: usize, y: usize, scale: usize, glyph: &Glyph, style: Style) {
        for (row, &bits) in glyph.iter().enumerate() {
            let bits = if style.bold { bits | (bits >> 1) } else { bits };
            for column in 0..WIDTH {
                if bits & (0x80 >> column) != 0 {
                    self.fill_rect(
                        x + column * scale,
                        y + row * scale,
                        scale,
                        scale,
                        style.color,
                    );
                }
            }
        }
    }

    /// Escreve `text` na linha que começa em `y`; devolve o `y` da linha seguinte.
    fn draw_text(
        &mut self,
        font: &Font,
        x: usize,
        y: usize,
        scale: usize,
        text: &str,
        style: Style,
    ) -> usize {
        self.grow_to(y + HEIGHT * scale);
        for (index, ch) in text.chars().enumerate() {
            self.draw_glyph(x + index * WIDTH * scale, y, scale, font.glyph(ch), style);
        }
        y + HEIGHT * scale
    }
}

#[derive(Clone, Copy)]
struct Style {
    color: u8,
    bold: bool,
}

const PLAIN: Style = Style {
    color: FOREGROUND,
    bold: false,
};
const BOLD: Style = Style {
    color: FOREGROUND,
    bold: true,
};
const HEADING_STYLE: Style = Style {
    color: HEADING,
    bold: false,
};
const LABEL_STYLE: Style = Style {
    color: LABEL,
    bold: false,
};

/// Quebra `text` em linhas de até `columns` caracteres, nos espaços.
fn wrap(text: &str, columns: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= columns => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
    }
    lines
}

/// Completa `lines` com espaços até a largura da mais longa.
fn pad_to_widest(lines: &[&str]) -> Vec<String> {
    let widest = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    lines
        .iter()
        .map(|line| format!("{line}{}", " ".repeat(widest - line.chars().count())))
        .collect()
}

/// Moldura de cantos arredondados em volta de `lines`, com um espaço de folga.
fn rounded_box(lines: &[&str]) -> Vec<String> {
    let body = pad_to_widest(lines);
    let inner = body.first().map_or(0, |line| line.chars().count()) + 2;
    let rule = "─".repeat(inner);
    let mut framed = vec![format!("╭{rule}╮")];
    framed.extend(body.iter().map(|line| format!("│ {line} │")));
    framed.push(format!("╰{rule}╯"));
    framed
}

fn render(font: &Font, scale: usize) -> Canvas {
    let cell_width = (WIDTH * scale).max(LABEL_WIDTH) + 2 * scale + 2;
    let width = MARGIN * 2 + GRID_COLUMNS * cell_width;
    let columns = (width - MARGIN * 2) / (WIDTH * scale);
    let mut canvas = Canvas::new(width);
    canvas.fill_rect(0, 0, width, 1, BACKGROUND);
    let mut y = MARGIN;
    let title = format!("RustOS font 8x16 - {} glifos", font.len());
    y = canvas.draw_text(font, MARGIN, y, scale, &title, HEADING_STYLE) + MARGIN;
    y = draw_grid(&mut canvas, font, scale, cell_width, y) + MARGIN;

    y = canvas.draw_text(font, MARGIN, y, scale, "ASCII e português", HEADING_STYLE);
    for text in [ASCII_SAMPLE, PANGRAM] {
        for line in wrap(text, columns) {
            y = canvas.draw_text(font, MARGIN, y, scale, &line, PLAIN);
        }
        y += scale * 2;
    }
    y = canvas.draw_text(
        font,
        MARGIN,
        y,
        scale,
        "Negrito (row | row >> 1)",
        HEADING_STYLE,
    );
    for line in wrap(ASCII_SAMPLE, columns) {
        y = canvas.draw_text(font, MARGIN, y, scale, &line, BOLD);
    }
    y += MARGIN;

    y = canvas.draw_text(font, MARGIN, y, scale, "Caixas", HEADING_STYLE);
    let boxed = rounded_box(&BOX_LINES);
    for line in boxed.iter().map(String::as_str).chain(TABLE) {
        y = canvas.draw_text(font, MARGIN, y, scale, line, PLAIN);
    }
    y += MARGIN;

    let big = scale * 2;
    let big_columns = (width - MARGIN * 2) / (WIDTH * big);
    y = canvas.draw_text(font, MARGIN, y, scale, "Escala 2x", HEADING_STYLE);
    for text in [ASCII_SAMPLE, PANGRAM] {
        for line in wrap(text, big_columns) {
            y = canvas.draw_text(font, MARGIN, y, big, &line, PLAIN);
        }
        y += big;
    }
    canvas.grow_to(y + MARGIN);
    canvas
}

/// Grade com todos os glifos em ordem de código, cada um com o código embaixo.
fn draw_grid(
    canvas: &mut Canvas,
    font: &Font,
    scale: usize,
    cell_width: usize,
    top: usize,
) -> usize {
    let cell_height = HEIGHT * scale + HEIGHT + 8;
    let glyph_width = WIDTH * scale;
    for (index, (&code, glyph)) in font.glyphs.iter().enumerate() {
        let x = MARGIN + (index % GRID_COLUMNS) * cell_width + (cell_width - glyph_width) / 2;
        let y = top + (index / GRID_COLUMNS) * cell_height;
        canvas.fill_rect(x, y, glyph_width, HEIGHT * scale, CELL);
        for guide_row in [3, 6, 12] {
            canvas.fill_rect(x, y + guide_row * scale, glyph_width, scale, GUIDE);
        }
        canvas.draw_glyph(x, y, scale, glyph, PLAIN);
        let label = format!("{code:04X}");
        let label_x = MARGIN + (index % GRID_COLUMNS) * cell_width + (cell_width - LABEL_WIDTH) / 2;
        canvas.draw_text(
            font,
            label_x,
            y + HEIGHT * scale + 2,
            1,
            &label,
            LABEL_STYLE,
        );
    }
    top + font.len().div_ceil(GRID_COLUMNS) * cell_height
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLANK: &str = "........\n";

    fn glyph_text(header: &str, rows: &str) -> String {
        let padding = BLANK.repeat(HEIGHT - rows.lines().count());
        format!("{header}\n{rows}{padding}")
    }

    fn tiny_font() -> Font {
        let text = format!(
            "; teste\n{}\n{}{}",
            glyph_text("U+0041 A", "#.......\n.#.....#\n"),
            glyph_text("U+0042", "########\n"),
            glyph_text("U+FFFD", "#######.\n")
        );
        Font::parse(&text).unwrap()
    }

    fn ink_count(canvas: &Canvas, color: u8) -> usize {
        canvas
            .pixels
            .iter()
            .map(|&pixel| usize::from(pixel == color))
            .sum()
    }

    fn workspace_root() -> std::path::PathBuf {
        crate::workspace::root()
    }

    #[test]
    fn parses_rows_with_column_zero_as_most_significant_bit() {
        let font = tiny_font();
        assert_eq!(font.len(), 3);
        assert_eq!(font.glyph('A')[..2], [0x80, 0x41]);
        assert_eq!(font.glyph('B')[0], 0xFF);
    }

    #[test]
    fn unknown_characters_use_the_replacement_glyph() {
        let font = tiny_font();
        assert_eq!(font.glyph('Z'), font.glyph('\u{FFFD}'));
        assert_eq!(font.glyph('\u{FFFD}')[0], 0xFE);
    }

    #[test]
    fn rejects_bad_files_with_the_line_number() {
        let wrong_width = glyph_text("U+0041", "#......\n");
        assert!(
            Font::parse(&wrong_width)
                .unwrap_err()
                .contains("glyphs.txt:2")
        );
        let bad_pixel = glyph_text("U+0041", "#...x...\n");
        assert!(
            Font::parse(&bad_pixel)
                .unwrap_err()
                .contains("glyphs.txt:2")
        );
        let truncated = "U+0041\n........\n";
        assert!(Font::parse(truncated).unwrap_err().contains("acaba"));
        let garbage = "lixo\n";
        assert!(Font::parse(garbage).unwrap_err().contains("glyphs.txt:1"));
        let duplicate = format!("{0}{0}", glyph_text("U+0041", ""));
        assert!(Font::parse(&duplicate).unwrap_err().contains("duplicado"));
        let no_replacement = glyph_text("U+0041", "");
        assert!(Font::parse(&no_replacement).unwrap_err().contains("U+FFFD"));
    }

    #[test]
    fn real_glyph_file_parses_and_covers_ascii_and_latin1() {
        let text = fs::read_to_string(workspace_root().join(GLYPHS_PATH)).unwrap();
        let font = Font::parse(&text).unwrap();
        for code in (0x20..=0x7E).chain(0xA0..=0xFF) {
            assert!(font.glyphs.contains_key(&code), "falta U+{code:04X}");
        }
    }

    #[test]
    fn scale_flag_defaults_and_validates() {
        let args = |list: &[&str]| list.iter().map(|&text| text.to_owned()).collect::<Vec<_>>();
        assert_eq!(parse_scale(&[]), Ok(DEFAULT_SCALE));
        assert_eq!(parse_scale(&args(&["--scale", "3"])), Ok(3));
        assert!(parse_scale(&args(&["--scale"])).is_err());
        assert!(parse_scale(&args(&["--scale", "0"])).is_err());
        assert!(parse_scale(&args(&["--scale", "9"])).is_err());
        assert!(parse_scale(&args(&["--scale", "x"])).is_err());
        assert!(parse_scale(&args(&["--arch", "x86_64"])).is_err());
    }

    #[test]
    fn draw_glyph_scales_pixels_and_bold_smears_right() {
        let mut canvas = Canvas::new(16);
        let mut glyph = [0u8; HEIGHT];
        glyph[0] = 0x80;
        canvas.draw_glyph(0, 0, 2, &glyph, PLAIN);
        let ink = |canvas: &Canvas| ink_count(canvas, FOREGROUND);
        assert_eq!(ink(&canvas), 4);
        assert_eq!(canvas.pixels[..4], [FOREGROUND, FOREGROUND, 0, 0]);
        let mut bold = Canvas::new(16);
        bold.draw_glyph(0, 0, 1, &glyph, BOLD);
        assert_eq!(bold.pixels[..3], [FOREGROUND, FOREGROUND, BACKGROUND]);
    }

    #[test]
    fn fill_rect_clips_at_the_right_edge_and_grows_down() {
        let mut canvas = Canvas::new(4);
        canvas.fill_rect(3, 1, 5, 2, CELL);
        assert_eq!(canvas.height, 3);
        assert_eq!(ink_count(&canvas, CELL), 2);
    }

    #[test]
    fn wrap_breaks_on_spaces_without_exceeding_the_width() {
        assert_eq!(wrap("aa bb cc", 5), ["aa bb", "cc"]);
        assert_eq!(wrap("aaaaaaa b", 4), ["aaaaaaa", "b"]);
        assert_eq!(wrap("", 4), [""]);
        assert_eq!(wrap("áé ç", 5), ["áé ç"]);
    }

    #[test]
    fn rounded_box_has_equal_width_rows_and_rounded_corners() {
        let framed = rounded_box(&["ab", "abcd"]);
        assert_eq!(framed, ["╭──────╮", "│ ab   │", "│ abcd │", "╰──────╯"]);
    }

    #[test]
    fn render_produces_an_image_that_encodes_to_png() {
        let font = tiny_font();
        let canvas = render(&font, 1);
        assert_eq!(canvas.pixels.len(), canvas.width * canvas.height);
        assert!(canvas.pixels.contains(&FOREGROUND) && canvas.pixels.contains(&CELL));
        assert!(png::encode_indexed(canvas.width, canvas.height, &PALETTE, &canvas.pixels).is_ok());
    }

    #[test]
    fn write_preview_creates_the_directory_and_a_png_file() {
        let output = std::env::temp_dir()
            .join(format!("rustos-font-preview-{}", std::process::id()))
            .join("nested/preview.png");
        let canvas = write_preview(&tiny_font(), 1, &output).unwrap();
        let bytes = fs::read(&output).unwrap();
        assert_eq!(
            &bytes[..8],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        );
        assert!(bytes.len() > canvas.width * canvas.height);
        let _ = fs::remove_dir_all(output.parent().and_then(Path::parent).unwrap());
    }

    #[test]
    fn run_reports_a_missing_glyph_file() {
        let error = run(Path::new("/nonexistent-rustos-root"), &[]).unwrap_err();
        assert!(error.contains("glyphs.txt"));
    }
}
