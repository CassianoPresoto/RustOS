//! Tela do kernel: liga a grade do `term` ao desenho do `gfx` no framebuffer.
//! Só as linhas sujas são redesenhadas; o framebuffer nunca é lido.

use core::{ptr, slice};

use rustos_gfx::{MAX_SCALE, Pixel, PixelFormat, Rect, Rgb, Style, Surface, font};
use rustos_hal::FramebufferInfo;
use rustos_term::{Cell, Flags, Terminal, xterm_rgb};

use crate::memory::Memory;

/// Tema escuro inspirado no Terminal do macOS: as 16 cores ANSI, normais e claras.
const ANSI: [Rgb; 16] = [
    Rgb::new(0x1C, 0x1C, 0x1E),
    Rgb::new(0xFF, 0x45, 0x3A),
    Rgb::new(0x30, 0xD1, 0x58),
    Rgb::new(0xFF, 0xD6, 0x0A),
    Rgb::new(0x0A, 0x84, 0xFF),
    Rgb::new(0xBF, 0x5A, 0xF2),
    Rgb::new(0x64, 0xD2, 0xFF),
    Rgb::new(0xD1, 0xD1, 0xD6),
    Rgb::new(0x63, 0x63, 0x66),
    Rgb::new(0xFF, 0x69, 0x61),
    Rgb::new(0x4C, 0xE0, 0x6A),
    Rgb::new(0xFF, 0xE0, 0x66),
    Rgb::new(0x40, 0x9C, 0xFF),
    Rgb::new(0xDA, 0x8F, 0xFF),
    Rgb::new(0x8E, 0xDC, 0xFF),
    Rgb::new(0xF5, 0xF5, 0xF7),
];
const BACKGROUND: Rgb = Rgb::new(0x1E, 0x1E, 0x1E);
const FOREGROUND: Rgb = Rgb::new(0xE5, 0xE5, 0xEA);
const BAR_BACKGROUND: Rgb = Rgb::new(0x2C, 0x2C, 0x2E);
const BAR_FOREGROUND: Rgb = Rgb::new(0xF2, 0xF2, 0xF7);
const BAR_DIM: Rgb = Rgb::new(0x98, 0x98, 0x9D);
const BAR_SEPARATOR: Rgb = Rgb::new(0x3A, 0x3A, 0x3C);

/// Altura da barra e margem da grade, em pixels antes da escala.
const BAR_HEIGHT: usize = 22;
const BAR_TEXT_TOP: usize = 3;
const PADDING: usize = 6;
/// Largura de tela que ainda usa escala 1; cada múltiplo dela soma 1 à escala.
const SCALE_STEP_WIDTH: usize = 1600;

#[derive(Debug)]
pub enum ScreenError {
    UnsupportedFormat,
    Misaligned,
    TooSmall,
    OutOfMemory,
}

#[derive(Clone, Copy, Debug)]
struct Palette {
    ansi: [Pixel; 16],
    format: PixelFormat,
    foreground: Pixel,
    background: Pixel,
}

impl Palette {
    fn new(format: PixelFormat) -> Self {
        Self {
            ansi: ANSI.map(|color| format.pack(color)),
            format,
            foreground: format.pack(FOREGROUND),
            background: format.pack(BACKGROUND),
        }
    }

    fn indexed(&self, index: u8) -> Pixel {
        self.ansi
            .get(usize::from(index))
            .copied()
            .unwrap_or_else(|| {
                let [r, g, b] = xterm_rgb(index);
                self.format.pack(Rgb::new(r, g, b))
            })
    }

    /// Cores finais da célula: padrão, negrito claro, inverso e cursor.
    fn colors(&self, cell: Cell, cursor: bool) -> (Pixel, Pixel) {
        let bold = cell.flags.contains(Flags::BOLD);
        let mut fg = if cell.flags.contains(Flags::DEFAULT_FG) {
            self.foreground
        } else if bold && cell.fg < 8 {
            self.indexed(cell.fg + 8)
        } else {
            self.indexed(cell.fg)
        };
        let mut bg = if cell.flags.contains(Flags::DEFAULT_BG) {
            self.background
        } else {
            self.indexed(cell.bg)
        };
        if cell.flags.contains(Flags::DIM) {
            fg = self.blend(fg, bg);
        }
        if cell.flags.contains(Flags::INVERSE) != cursor {
            core::mem::swap(&mut fg, &mut bg);
        }
        (fg, bg)
    }

    fn blend(&self, a: Pixel, b: Pixel) -> Pixel {
        let (a, b) = (self.format.unpack(a), self.format.unpack(b));
        self.format.pack(Rgb::new(
            u8::midpoint(a.r, b.r),
            u8::midpoint(a.g, b.g),
            u8::midpoint(a.b, b.b),
        ))
    }
}

#[derive(Debug)]
pub struct Screen {
    surface: Surface<'static>,
    terminal: Terminal<'static>,
    palette: Palette,
    scale: usize,
    grid_x: usize,
    grid_y: usize,
}

impl Screen {
    /// # Safety
    /// Chamar uma vez: `info` veio do bootloader e descreve um framebuffer mapeado
    /// que só esta tela vai tocar.
    pub unsafe fn new(info: &FramebufferInfo, memory: &mut Memory) -> Result<Self, ScreenError> {
        let format = PixelFormat::new(info.red.shift, info.green.shift, info.blue.shift)
            .ok_or(ScreenError::UnsupportedFormat)?;
        if !info
            .address
            .is_aligned(u64::from(FramebufferInfo::BYTES_PER_PIXEL))
        {
            return Err(ScreenError::Misaligned);
        }
        let (width, height) = (to_usize(info.width), to_usize(info.height));
        let stride = to_usize(info.stride_pixels());
        let len = to_usize_u64(info.len_bytes() / u64::from(FramebufferInfo::BYTES_PER_PIXEL));
        let start = ptr::with_exposed_provenance_mut::<u32>(to_usize_u64(info.address.as_u64()));
        // SAFETY: memória do framebuffer, alinhada e exclusiva desta tela (contrato da função).
        let pixels = unsafe { slice::from_raw_parts_mut(start, len) };
        let surface =
            Surface::new(pixels, width, height, stride, format).ok_or(ScreenError::TooSmall)?;

        let scale = (width / SCALE_STEP_WIDTH + 1).min(MAX_SCALE);
        let (cell_w, cell_h) = (font::WIDTH * scale, font::HEIGHT * scale);
        let (grid_x, grid_y) = (PADDING * scale, (BAR_HEIGHT + PADDING) * scale);
        let cols = width.saturating_sub(2 * grid_x) / cell_w;
        let rows = height.saturating_sub(grid_y + PADDING * scale) / cell_h;
        let rows = rows.min(rustos_term::MAX_ROWS);
        let cells = memory
            .alloc_slice(Terminal::cells_needed(cols, rows), Cell::BLANK)
            .ok_or(ScreenError::OutOfMemory)?;
        let terminal = Terminal::new(cells, cols, rows).ok_or(ScreenError::TooSmall)?;

        let mut screen = Self {
            surface,
            terminal,
            palette: Palette::new(format),
            scale,
            grid_x,
            grid_y,
        };
        screen.surface.fill(screen.palette.background);
        screen.draw_bar();
        Ok(screen)
    }

    pub fn size(&self) -> (usize, usize) {
        (self.surface.width(), self.surface.height())
    }

    pub fn grid(&self) -> (usize, usize) {
        (self.terminal.cols(), self.terminal.rows())
    }

    /// Texto do kernel e do shell; `\n` vira `\r\n`, como o `onlcr` de um tty.
    /// Só atualiza a grade: os pixels mudam no próximo `render`.
    pub fn write(&mut self, bytes: &[u8]) {
        for (index, line) in bytes.split(|&byte| byte == b'\n').enumerate() {
            if index > 0 {
                self.terminal.write(b"\r\n");
            }
            self.terminal.write(line);
        }
        let mut discard = [0; 32];
        while self.terminal.take_reply(&mut discard) > 0 {}
    }

    /// Desenha as linhas sujas desde o último `render`. Rolar suja a grade inteira.
    pub fn render(&mut self) {
        let cursor = self.terminal.cursor();
        let (cell_w, cell_h) = (font::WIDTH * self.scale, font::HEIGHT * self.scale);
        for row in self.terminal.take_dirty() {
            let y = self.grid_y + row * cell_h;
            for (col, &cell) in self.terminal.row(row).iter().enumerate() {
                let on_cursor = cursor.visible && cursor.row == row && cursor.col == col;
                let (fg, bg) = self.palette.colors(cell, on_cursor);
                let style = Style {
                    bold: cell.flags.contains(Flags::BOLD),
                    underline: cell.flags.contains(Flags::UNDERLINE),
                    strike: cell.flags.contains(Flags::STRIKE),
                };
                let rows = style.apply(font::glyph(cell.ch));
                let x = self.grid_x + col * cell_w;
                self.surface.draw_glyph(x, y, &rows, self.scale, fg, bg);
            }
        }
    }

    /// Barra no topo, no estilo da barra de menus do macOS.
    fn draw_bar(&mut self) {
        let format = self.palette.format;
        let (width, scale) = (self.surface.width(), self.scale);
        let bar = format.pack(BAR_BACKGROUND);
        self.surface
            .fill_rect(Rect::new(0, 0, width, BAR_HEIGHT * scale), bar);
        self.surface.fill_rect(
            Rect::new(0, BAR_HEIGHT * scale - scale, width, scale),
            format.pack(BAR_SEPARATOR),
        );
        let y = BAR_TEXT_TOP * scale;
        let bright = format.pack(BAR_FOREGROUND);
        let dim = format.pack(BAR_DIM);
        let bold = Style {
            bold: true,
            ..Style::PLAIN
        };
        let mut x = PADDING * 2 * scale;
        x = self.draw_text(x, y, "\u{25CF} RustOS", bold, bright, bar);
        x += font::WIDTH * scale * 2;
        self.draw_text(x, y, "Terminal", Style::PLAIN, dim, bar);

        let mut info = crate::fmt::Buffer::<48>::new();
        core::fmt::write(
            &mut info,
            format_args!(
                "{}  \u{00B7}  {}\u{00D7}{}",
                crate::platform::ARCH,
                width,
                self.surface.height()
            ),
        )
        .ok();
        let text = info.as_str();
        let text_w = text.chars().count() * font::WIDTH * scale;
        let right = width.saturating_sub(text_w + PADDING * 2 * scale);
        self.draw_text(right, y, text, Style::PLAIN, dim, bar);
    }

    fn draw_text(
        &mut self,
        x: usize,
        y: usize,
        text: &str,
        style: Style,
        fg: Pixel,
        bg: Pixel,
    ) -> usize {
        let advance = font::WIDTH * self.scale;
        let mut x = x;
        for ch in text.chars() {
            let rows = style.apply(font::glyph(ch));
            self.surface.draw_glyph(x, y, &rows, self.scale, fg, bg);
            x += advance;
        }
        x
    }
}

fn to_usize(value: u32) -> usize {
    usize::try_from(value).expect("u32 cabe em usize nos alvos de 64 bits")
}

fn to_usize_u64(value: u64) -> usize {
    usize::try_from(value).expect("endereço cabe em usize nos alvos de 64 bits")
}
