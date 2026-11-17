//! O emulador: executa o que o parser reconhece sobre a grade de células.

use crate::cell::{Cell, Flags};
use crate::charset::Charset;
use crate::dirty::{DirtyRows, MAX_ROWS};
use crate::history::History;
use crate::key::{self, Key};
use crate::palette::nearest_index;
use crate::parser::{Event, Parser};
use crate::tabs::{MAX_COLS, TabStops};

const REPLY_CAP: usize = 64;

const MODE_APP_CURSOR: u8 = 1;
const MODE_ORIGIN: u8 = 2;
const MODE_AUTOWRAP: u8 = 4;
const MODE_CURSOR_VISIBLE: u8 = 8;

/// Posição do cursor na grade, já dentro dos limites.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    /// Linha, a partir de 0.
    pub row: usize,
    /// Coluna, a partir de 0. Com quebra pendente continua na última coluna.
    pub col: usize,
    /// DECTCEM (`?25`): o renderizador só desenha o cursor se for `true`.
    pub visible: bool,
}

#[derive(Clone, Copy, Debug)]
struct Pen {
    fg: u8,
    bg: u8,
    flags: Flags,
}

impl Pen {
    const DEFAULT: Pen = Pen {
        fg: 7,
        bg: 0,
        flags: Flags::DEFAULT_FG.union(Flags::DEFAULT_BG),
    };
}

#[derive(Clone, Copy, Debug)]
struct Saved {
    row: usize,
    col: usize,
    pen: Pen,
    origin: bool,
    wrap_pending: bool,
    sets: [Charset; 2],
    gl: usize,
}

impl Saved {
    const INITIAL: Saved = Saved {
        row: 0,
        col: 0,
        pen: Pen::DEFAULT,
        origin: false,
        wrap_pending: false,
        sets: [Charset::Ascii; 2],
        gl: 0,
    };
}

/// Emulador VT/xterm sobre uma grade de células cuja memória vem de fora.
#[derive(Debug)]
pub struct Terminal<'a> {
    cells: &'a mut [Cell],
    other: &'a mut [Cell],
    other_ring: usize,
    alt_active: bool,
    history: History<'a>,
    view: usize,
    cols: usize,
    rows: usize,
    ring: usize,
    row: usize,
    col: usize,
    wrap_pending: bool,
    pen: Pen,
    saved: [Saved; 2],
    sets: [Charset; 2],
    gl: usize,
    tabs: TabStops,
    scroll_top: usize,
    scroll_bottom: usize,
    modes: u8,
    parser: Parser,
    dirty: DirtyRows,
    last_cursor: Cursor,
    reply: [u8; REPLY_CAP],
    reply_len: usize,
    bell: bool,
    last_char: Option<char>,
}

fn is_zero_width(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2060}'..='\u{2064}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FE20}'..='\u{FE2F}'
            | '\u{FEFF}'
            | '\u{E0100}'..='\u{E01EF}'
    )
}

#[allow(clippy::cast_possible_truncation, reason = "dígito decimal cabe em u8")]
fn put_number(buf: &mut [u8], mut pos: usize, value: usize) -> usize {
    let mut digits = [0u8; 20];
    let mut len = 0;
    let mut rest = value;
    loop {
        digits[len] = b'0' + (rest % 10) as u8;
        len += 1;
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    while len > 0 {
        len -= 1;
        buf[pos] = digits[len];
        pos += 1;
    }
    pos
}

impl<'a> Terminal<'a> {
    /// Quantas células `new` exige para `cols` x `rows`; satura em vez de transbordar.
    pub const fn cells_needed(cols: usize, rows: usize) -> usize {
        cols.saturating_mul(rows)
    }

    /// Quantas células o histórico precisa para guardar `lines` linhas de `cols` colunas.
    pub const fn history_cells_needed(cols: usize, lines: usize) -> usize {
        cols.saturating_mul(lines)
    }

    /// Cria o terminal sobre `cells`; `None` se a fatia for curta, o tamanho for 0,
    /// `rows` passar de `MAX_ROWS` ou `cols` de `MAX_COLS`. A grade começa em branco e
    /// toda suja, sem tela alternativa nem histórico (`with_alternate`, `with_history`).
    pub fn new(cells: &'a mut [Cell], cols: usize, rows: usize) -> Option<Self> {
        let needed = Self::cells_needed(cols, rows);
        if cols == 0 || rows == 0 || rows > MAX_ROWS || cols > MAX_COLS || cells.len() < needed {
            return None;
        }
        cells[..needed].fill(Cell::BLANK);
        let cursor = Cursor {
            row: 0,
            col: 0,
            visible: true,
        };
        let mut dirty = DirtyRows::default();
        dirty.mark_range(0, rows);
        Some(Terminal {
            cells,
            other: &mut [],
            other_ring: 0,
            alt_active: false,
            history: History::new(&mut [], cols),
            view: 0,
            cols,
            rows,
            ring: 0,
            row: 0,
            col: 0,
            wrap_pending: false,
            pen: Pen::DEFAULT,
            saved: [Saved::INITIAL; 2],
            sets: [Charset::Ascii; 2],
            gl: 0,
            tabs: TabStops::every_eight(),
            scroll_top: 0,
            scroll_bottom: rows - 1,
            modes: MODE_AUTOWRAP | MODE_CURSOR_VISIBLE,
            parser: Parser::new(),
            dirty,
            last_cursor: cursor,
            reply: [0; REPLY_CAP],
            reply_len: 0,
            bell: false,
            last_char: None,
        })
    }

    /// Entrega a memória da tela alternativa (`?47`, `?1047`, `?1049`): `cells_needed`
    /// células, que passam a branco. `None` se a fatia for curta. Sem ela esses modos
    /// não fazem nada.
    pub fn with_alternate(mut self, alt: &'a mut [Cell]) -> Option<Self> {
        let needed = Self::cells_needed(self.cols, self.rows);
        if alt.len() < needed {
            return None;
        }
        alt[..needed].fill(Cell::BLANK);
        self.other = alt;
        Some(self)
    }

    /// Entrega a memória do histórico de rolagem: cabem `len / cols` linhas, e a mais
    /// antiga sai quando enche. Fatia menor que uma linha deixa o histórico desligado.
    #[must_use]
    pub fn with_history(mut self, cells: &'a mut [Cell]) -> Self {
        self.history = History::new(cells, self.cols);
        self
    }

    /// Número de colunas.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Número de linhas.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// A linha `row` da visão (0 é a de cima), com o histórico se a visão estiver
    /// rolada; fatia vazia se `row` estiver fora da grade.
    pub fn row(&self, row: usize) -> &[Cell] {
        if row >= self.rows {
            return &[];
        }
        if self.view > 0 {
            let index = self.history.len() + row - self.view;
            return match self.history.get(index) {
                Some(line) => line,
                None => &self.cells[self.row_range(index - self.history.len())],
            };
        }
        &self.cells[self.row_range(row)]
    }

    /// Posição e visibilidade do cursor; some com a visão rolada para trás.
    pub fn cursor(&self) -> Cursor {
        Cursor {
            row: self.row,
            col: self.col,
            visible: self.modes & MODE_CURSOR_VISIBLE != 0 && self.view == 0,
        }
    }

    /// A tela alternativa está ativa.
    pub fn is_alternate(&self) -> bool {
        self.alt_active
    }

    /// Linhas guardadas no histórico.
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Quantas linhas o histórico comporta.
    pub fn history_capacity(&self) -> usize {
        self.history.capacity()
    }

    /// Quantas linhas a visão está acima da tela viva; 0 é a tela viva.
    pub fn view_offset(&self) -> usize {
        self.view
    }

    /// Rola a visão `lines` linhas para trás, até o começo do histórico. Na tela
    /// alternativa não há histórico e nada acontece.
    pub fn scroll_view_back(&mut self, lines: usize) {
        if !self.alt_active {
            self.set_view(self.view.saturating_add(lines));
        }
    }

    /// Rola a visão `lines` linhas para a frente, até a tela viva.
    pub fn scroll_view_forward(&mut self, lines: usize) {
        self.set_view(self.view.saturating_sub(lines));
    }

    /// Volta a visão para a tela viva.
    pub fn snap_to_live(&mut self) {
        self.set_view(0);
    }

    fn set_view(&mut self, view: usize) {
        let view = view.min(self.history.len());
        if view != self.view {
            self.view = view;
            self.dirty.mark_range(0, self.rows);
        }
    }

    /// Linhas que mudaram desde a última chamada, incluindo a antiga e a nova linha do
    /// cursor se ele andou ou mudou de visibilidade. Zera o conjunto.
    pub fn take_dirty(&mut self) -> DirtyRows {
        let now = self.cursor();
        if now != self.last_cursor {
            self.dirty.mark(self.last_cursor.row);
            self.dirty.mark(now.row);
            self.last_cursor = now;
        }
        self.dirty.drain()
    }

    /// Copia para `out` os bytes que o terminal devolve ao programa (respostas a DSR e DA)
    /// e os remove da fila. Devolve quantos copiou.
    pub fn take_reply(&mut self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.reply_len);
        out[..n].copy_from_slice(&self.reply[..n]);
        self.reply.copy_within(n..self.reply_len, 0);
        self.reply_len -= n;
        n
    }

    /// `true` se um BEL chegou desde a última chamada; zera o sinal.
    pub fn take_bell(&mut self) -> bool {
        core::mem::take(&mut self.bell)
    }

    /// Bytes da tecla para o programa; respeita DECCKM nas setas, Home e End.
    pub fn encode_key(&self, key: Key, out: &mut [u8; 8]) -> usize {
        key::encode(key, self.modes & MODE_APP_CURSOR != 0, out)
    }

    /// Alimenta o terminal. O estado do parser atravessa chamadas: UTF-8 e sequências
    /// podem ser cortados em qualquer byte.
    pub fn write(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.snap_to_live();
        }
        let mut i = 0;
        while i < bytes.len() {
            if self.parser.is_idle() && self.sets[self.gl] == Charset::Ascii {
                let run = bytes[i..]
                    .iter()
                    .take_while(|&&b| (0x20..0x7F).contains(&b))
                    .count();
                if run > 0 {
                    self.print_ascii(&bytes[i..i + run]);
                    i += run;
                    continue;
                }
            }
            self.feed(bytes[i]);
            i += 1;
        }
    }

    fn feed(&mut self, byte: u8) {
        let event = self.parser.advance(byte);
        self.apply(event);
        if let Some(again) = self.parser.take_replay() {
            let event = self.parser.advance(again);
            self.apply(event);
        }
    }

    fn apply(&mut self, event: Event) {
        match event {
            Event::None => {}
            Event::Print(c) => self.print_char(c),
            Event::Execute(byte) => self.execute(byte),
            Event::Esc(byte) => self.esc(byte),
            Event::Csi(byte) => self.csi(byte),
        }
    }

    fn row_range(&self, logical: usize) -> core::ops::Range<usize> {
        let mut physical = self.ring + logical;
        if physical >= self.rows {
            physical -= self.rows;
        }
        physical * self.cols..(physical + 1) * self.cols
    }

    fn blank(&self) -> Cell {
        let mut flags = Flags::DEFAULT_FG;
        if self.pen.flags.contains(Flags::DEFAULT_BG) {
            flags.insert(Flags::DEFAULT_BG);
        }
        Cell {
            ch: ' ',
            fg: Pen::DEFAULT.fg,
            bg: self.pen.bg,
            flags,
        }
    }

    fn cell(&self, ch: char) -> Cell {
        Cell {
            ch,
            fg: self.pen.fg,
            bg: self.pen.bg,
            flags: self.pen.flags,
        }
    }

    fn arg(&self, index: usize, default: usize) -> usize {
        match self.parser.param(index) {
            0 => default,
            value => usize::from(value),
        }
    }

    fn origin(&self) -> bool {
        self.modes & MODE_ORIGIN != 0
    }

    fn autowrap(&self) -> bool {
        self.modes & MODE_AUTOWRAP != 0
    }

    // ---- texto ----

    fn alt_available(&self) -> bool {
        self.alt_active || self.other.len() >= Self::cells_needed(self.cols, self.rows)
    }

    fn print_ascii(&mut self, mut text: &[u8]) {
        while let Some(&last) = text.last() {
            if self.wrap_pending {
                self.resolve_wrap();
            }
            let n = text.len().min(self.cols - self.col);
            let start = self.row_range(self.row).start + self.col;
            let (fg, bg, flags) = (self.pen.fg, self.pen.bg, self.pen.flags);
            for (cell, &byte) in self.cells[start..start + n].iter_mut().zip(text) {
                *cell = Cell {
                    ch: char::from(byte),
                    fg,
                    bg,
                    flags,
                };
            }
            self.dirty.mark(self.row);
            self.last_char = Some(char::from(last));
            text = &text[n..];
            self.col += n;
            if self.col >= self.cols {
                self.col = self.cols - 1;
                self.wrap_pending = self.autowrap();
            }
        }
    }

    fn print_char(&mut self, c: char) {
        let c = self.sets[self.gl].map(c);
        if is_zero_width(c) {
            return;
        }
        if self.wrap_pending {
            self.resolve_wrap();
        }
        let at = self.row_range(self.row).start + self.col;
        self.cells[at] = self.cell(c);
        self.dirty.mark(self.row);
        self.last_char = Some(c);
        if self.col + 1 < self.cols {
            self.col += 1;
        } else {
            self.wrap_pending = self.autowrap();
        }
    }

    fn resolve_wrap(&mut self) {
        self.wrap_pending = false;
        if self.autowrap() {
            self.col = 0;
            self.index();
        }
    }

    // ---- controles ----

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => self.bell = true,
            0x08 => self.set_col(self.col.saturating_sub(1)),
            0x09 => self.tab_forward(1),
            0x0E => self.gl = 1,
            0x0F => self.gl = 0,
            0x0A..=0x0C => {
                self.wrap_pending = false;
                self.index();
            }
            0x0D => self.set_col(0),
            _ => {}
        }
    }

    fn esc(&mut self, byte: u8) {
        let intermediate = self.parser.intermediate();
        if intermediate != 0 {
            let slot = usize::from(intermediate == b')');
            if matches!(intermediate, b'(' | b')') {
                self.sets[slot] = Charset::from_final(byte);
            }
            return;
        }
        match byte {
            b'H' => self.tabs.set(self.col),
            b'7' => self.save_cursor(),
            b'8' => self.restore_cursor(),
            b'D' => {
                self.wrap_pending = false;
                self.index();
            }
            b'E' => {
                self.set_col(0);
                self.index();
            }
            b'M' => {
                self.wrap_pending = false;
                self.reverse_index();
            }
            b'c' => self.reset(),
            _ => {}
        }
    }

    fn reset(&mut self) {
        if self.alt_active {
            self.cells[..self.cols * self.rows].fill(Cell::BLANK);
            self.switch_screen(false);
        }
        self.pen = Pen::DEFAULT;
        self.saved = [Saved::INITIAL; 2];
        self.sets = [Charset::Ascii; 2];
        self.gl = 0;
        self.tabs = TabStops::every_eight();
        self.history.clear();
        self.view = 0;
        self.modes = MODE_AUTOWRAP | MODE_CURSOR_VISIBLE;
        self.scroll_top = 0;
        self.scroll_bottom = self.rows - 1;
        self.ring = 0;
        self.row = 0;
        self.col = 0;
        self.wrap_pending = false;
        self.last_char = None;
        self.parser = Parser::new();
        self.cells[..self.cols * self.rows].fill(Cell::BLANK);
        self.dirty.mark_range(0, self.rows);
    }

    fn save_cursor(&mut self) {
        self.saved[usize::from(self.alt_active)] = Saved {
            row: self.row,
            col: self.col,
            pen: self.pen,
            origin: self.origin(),
            wrap_pending: self.wrap_pending,
            sets: self.sets,
            gl: self.gl,
        };
    }

    fn restore_cursor(&mut self) {
        let saved = self.saved[usize::from(self.alt_active)];
        self.set_mode(MODE_ORIGIN, saved.origin);
        self.row = if saved.origin {
            saved.row.clamp(self.scroll_top, self.scroll_bottom)
        } else {
            saved.row.min(self.rows - 1)
        };
        self.col = saved.col.min(self.cols - 1);
        self.pen = saved.pen;
        self.sets = saved.sets;
        self.gl = saved.gl;
        self.wrap_pending = saved.wrap_pending;
    }

    fn switch_screen(&mut self, alternate: bool) {
        let ready = !alternate || self.alt_available();
        if alternate == self.alt_active || !ready {
            return;
        }
        core::mem::swap(&mut self.cells, &mut self.other);
        core::mem::swap(&mut self.ring, &mut self.other_ring);
        self.alt_active = alternate;
        self.view = 0;
        self.dirty.mark_range(0, self.rows);
    }

    fn clear_screen(&mut self) {
        self.wrap_pending = false;
        self.fill_rows(0, self.rows);
    }

    // ---- movimento ----

    fn set_col(&mut self, col: usize) {
        self.col = col.min(self.cols - 1);
        self.wrap_pending = false;
    }

    fn tab_forward(&mut self, count: usize) {
        for _ in 0..count {
            let next = self.tabs.next(self.col, self.cols - 1);
            if next == self.col {
                break;
            }
            self.col = next;
        }
    }

    fn tab_backward(&mut self, count: usize) {
        for _ in 0..count {
            let prev = self.tabs.prev(self.col);
            if prev == self.col {
                break;
            }
            self.col = prev;
        }
    }

    fn index(&mut self) {
        if self.row == self.scroll_bottom {
            self.scroll_up(self.scroll_top, self.scroll_bottom, 1);
        } else if self.row + 1 < self.rows {
            self.row += 1;
        }
    }

    fn reverse_index(&mut self) {
        if self.row == self.scroll_top {
            self.scroll_down(self.scroll_top, self.scroll_bottom, 1);
        } else if self.row > 0 {
            self.row -= 1;
        }
    }

    fn move_up(&mut self, n: usize) {
        let limit = if self.row >= self.scroll_top {
            self.scroll_top
        } else {
            0
        };
        self.row = self.row.saturating_sub(n).max(limit);
        self.wrap_pending = false;
    }

    fn move_down(&mut self, n: usize) {
        let limit = if self.row <= self.scroll_bottom {
            self.scroll_bottom
        } else {
            self.rows - 1
        };
        self.row = self.row.saturating_add(n).min(limit);
        self.wrap_pending = false;
    }

    fn goto_row(&mut self, row: usize) {
        self.row = if self.origin() {
            self.scroll_top.saturating_add(row).min(self.scroll_bottom)
        } else {
            row.min(self.rows - 1)
        };
        self.wrap_pending = false;
    }

    fn goto_home(&mut self) {
        self.goto_row(0);
        self.set_col(0);
    }

    // ---- rolagem e edição ----

    fn copy_row(&mut self, from: usize, to: usize) {
        let src = self.row_range(from).start;
        let dst = self.row_range(to).start;
        self.cells.copy_within(src..src + self.cols, dst);
    }

    fn fill_rows(&mut self, from: usize, to: usize) {
        let blank = self.blank();
        for row in from..to {
            let range = self.row_range(row);
            self.cells[range].fill(blank);
        }
        self.dirty.mark_range(from, to);
    }

    fn scroll_up(&mut self, top: usize, bottom: usize, n: usize) {
        let n = n.min(bottom - top + 1);
        if top == 0 && !self.alt_active {
            for row in 0..n {
                let range = self.row_range(row);
                self.history.push(&self.cells[range]);
            }
        }
        if top == 0 && bottom + 1 == self.rows {
            self.ring = (self.ring + n) % self.rows;
            self.fill_rows(self.rows - n, self.rows);
            self.dirty.mark_range(0, self.rows);
        } else {
            for row in top..bottom + 1 - n {
                self.copy_row(row + n, row);
            }
            self.fill_rows(bottom + 1 - n, bottom + 1);
            self.dirty.mark_range(top, bottom + 1);
        }
    }

    fn scroll_down(&mut self, top: usize, bottom: usize, n: usize) {
        let n = n.min(bottom - top + 1);
        if top == 0 && bottom + 1 == self.rows {
            self.ring = (self.ring + self.rows - n) % self.rows;
            self.fill_rows(0, n);
            self.dirty.mark_range(0, self.rows);
        } else {
            for row in (top + n..=bottom).rev() {
                self.copy_row(row - n, row);
            }
            self.fill_rows(top, top + n);
            self.dirty.mark_range(top, bottom + 1);
        }
    }

    fn insert_lines(&mut self, n: usize) {
        if (self.scroll_top..=self.scroll_bottom).contains(&self.row) {
            self.scroll_down(self.row, self.scroll_bottom, n);
            self.set_col(0);
        }
    }

    fn delete_lines(&mut self, n: usize) {
        if (self.scroll_top..=self.scroll_bottom).contains(&self.row) {
            self.scroll_up(self.row, self.scroll_bottom, n);
            self.set_col(0);
        }
    }

    fn fill_cells(&mut self, row: usize, from: usize, to: usize) {
        let blank = self.blank();
        let start = self.row_range(row).start;
        self.cells[start + from..start + to].fill(blank);
        self.dirty.mark(row);
    }

    fn erase_display(&mut self, mode: u16) {
        if mode > 3 {
            return;
        }
        if mode == 3 {
            self.history.clear();
            self.set_view(0);
            return;
        }
        self.wrap_pending = false;
        match mode {
            0 => {
                self.fill_cells(self.row, self.col, self.cols);
                self.fill_rows(self.row + 1, self.rows);
            }
            1 => {
                self.fill_rows(0, self.row);
                self.fill_cells(self.row, 0, self.col + 1);
            }
            _ => self.fill_rows(0, self.rows),
        }
    }

    fn erase_line(&mut self, mode: u16) {
        if mode > 2 {
            return;
        }
        self.wrap_pending = false;
        match mode {
            0 => self.fill_cells(self.row, self.col, self.cols),
            1 => self.fill_cells(self.row, 0, self.col + 1),
            _ => self.fill_cells(self.row, 0, self.cols),
        }
    }

    fn insert_chars(&mut self, n: usize) {
        self.wrap_pending = false;
        let n = n.min(self.cols - self.col);
        let start = self.row_range(self.row).start;
        let row = &mut self.cells[start..start + self.cols];
        row.copy_within(self.col..self.cols - n, self.col + n);
        self.fill_cells(self.row, self.col, self.col + n);
    }

    fn delete_chars(&mut self, n: usize) {
        self.wrap_pending = false;
        let n = n.min(self.cols - self.col);
        let start = self.row_range(self.row).start;
        let row = &mut self.cells[start..start + self.cols];
        row.copy_within(self.col + n..self.cols, self.col);
        self.fill_cells(self.row, self.cols - n, self.cols);
    }

    fn erase_chars(&mut self, n: usize) {
        self.wrap_pending = false;
        self.fill_cells(self.row, self.col, (self.col + n).min(self.cols));
    }

    // ---- CSI ----

    fn csi(&mut self, byte: u8) {
        if self.parser.intermediate() != 0 {
            return;
        }
        match self.parser.marker() {
            0 => self.csi_standard(byte),
            b'?' => match byte {
                b'h' => self.set_private_modes(true),
                b'l' => self.set_private_modes(false),
                _ => {}
            },
            _ => {}
        }
    }

    fn csi_standard(&mut self, byte: u8) {
        let n = self.arg(0, 1);
        match byte {
            b'A' => self.move_up(n),
            b'B' | b'e' => self.move_down(n),
            b'C' | b'a' => self.set_col(self.col.saturating_add(n)),
            b'D' => self.set_col(self.col.saturating_sub(n)),
            b'E' => {
                self.move_down(n);
                self.set_col(0);
            }
            b'F' => {
                self.move_up(n);
                self.set_col(0);
            }
            b'G' | b'`' => self.set_col(n - 1),
            b'd' => self.goto_row(n - 1),
            b'H' | b'f' => {
                self.goto_row(n - 1);
                self.set_col(self.arg(1, 1) - 1);
            }
            b'J' => self.erase_display(self.parser.param(0)),
            b'K' => self.erase_line(self.parser.param(0)),
            b'L' => self.insert_lines(n),
            b'M' => self.delete_lines(n),
            b'@' => self.insert_chars(n),
            b'P' => self.delete_chars(n),
            b'X' => self.erase_chars(n),
            b'S' => self.scroll_up(self.scroll_top, self.scroll_bottom, n),
            b'T' => self.scroll_down(self.scroll_top, self.scroll_bottom, n),
            b'b' => self.repeat_last(n),
            b'I' => self.tab_forward(n),
            b'Z' => self.tab_backward(n),
            b'g' => self.tab_clear(),
            b'r' => self.set_scroll_region(),
            b's' => self.save_cursor(),
            b'u' => self.restore_cursor(),
            b'm' => self.sgr(),
            b'n' => self.device_status(),
            b'c' if self.parser.param(0) == 0 => self.push_reply(b"\x1b[?6c"),
            _ => {}
        }
    }

    fn repeat_last(&mut self, n: usize) {
        if let Some(c) = self.last_char {
            for _ in 0..n {
                self.print_char(c);
            }
        }
    }

    fn tab_clear(&mut self) {
        match self.parser.param(0) {
            0 => self.tabs.clear(self.col),
            3 => self.tabs.clear_all(),
            _ => {}
        }
    }

    fn set_scroll_region(&mut self) {
        let top = self.arg(0, 1);
        let bottom = self.arg(1, self.rows).min(self.rows);
        if top < bottom {
            self.scroll_top = top - 1;
            self.scroll_bottom = bottom - 1;
            self.goto_home();
        }
    }

    fn set_mode(&mut self, bit: u8, on: bool) {
        if on {
            self.modes |= bit;
        } else {
            self.modes &= !bit;
        }
    }

    fn set_private_modes(&mut self, on: bool) {
        for i in 0..self.parser.param_count() {
            match self.parser.param(i) {
                1 => self.set_mode(MODE_APP_CURSOR, on),
                6 => {
                    self.set_mode(MODE_ORIGIN, on);
                    self.goto_home();
                }
                7 => self.set_mode(MODE_AUTOWRAP, on),
                25 => self.set_mode(MODE_CURSOR_VISIBLE, on),
                47 => self.switch_screen(on),
                1047 => {
                    if !on && self.alt_active {
                        self.clear_screen();
                    }
                    self.switch_screen(on);
                }
                1048 if on => self.save_cursor(),
                1048 => self.restore_cursor(),
                1049 if on => {
                    self.save_cursor();
                    self.switch_screen(true);
                    if self.alt_active {
                        self.clear_screen();
                    }
                }
                1049 => {
                    self.switch_screen(false);
                    self.restore_cursor();
                }
                _ => {}
            }
        }
    }

    // ---- SGR ----

    fn sgr(&mut self) {
        let count = self.parser.param_count();
        if count == 0 {
            self.pen = Pen::DEFAULT;
            return;
        }
        let mut i = 0;
        while i < count {
            let code = self.parser.param(i);
            i += 1;
            match code {
                38 | 48 => i += self.sgr_extended(i, count, code == 38),
                _ => self.sgr_simple(code),
            }
        }
    }

    fn sgr_simple(&mut self, code: u16) {
        let flags = &mut self.pen.flags;
        match code {
            0 => self.pen = Pen::DEFAULT,
            1 => flags.insert(Flags::BOLD),
            2 => flags.insert(Flags::DIM),
            3 => flags.insert(Flags::ITALIC),
            4 => flags.insert(Flags::UNDERLINE),
            7 => flags.insert(Flags::INVERSE),
            9 => flags.insert(Flags::STRIKE),
            22 => flags.remove(Flags::BOLD.union(Flags::DIM)),
            23 => flags.remove(Flags::ITALIC),
            24 => flags.remove(Flags::UNDERLINE),
            27 => flags.remove(Flags::INVERSE),
            29 => flags.remove(Flags::STRIKE),
            39 => self.pen.flags.insert(Flags::DEFAULT_FG),
            49 => self.pen.flags.insert(Flags::DEFAULT_BG),
            30..=37 => self.set_color(true, code - 30),
            90..=97 => self.set_color(true, code - 90 + 8),
            40..=47 => self.set_color(false, code - 40),
            100..=107 => self.set_color(false, code - 100 + 8),
            _ => {}
        }
    }

    fn set_color(&mut self, foreground: bool, index: u16) {
        let Ok(index) = u8::try_from(index) else {
            return;
        };
        if foreground {
            self.pen.fg = index;
            self.pen.flags.remove(Flags::DEFAULT_FG);
        } else {
            self.pen.bg = index;
            self.pen.flags.remove(Flags::DEFAULT_BG);
        }
    }

    fn sgr_extended(&mut self, at: usize, count: usize, foreground: bool) -> usize {
        let available = count - at;
        match self.parser.param(at) {
            5 if available >= 2 => {
                self.set_color(foreground, self.parser.param(at + 1));
                2
            }
            2 if available >= 4 => {
                let channel = |i: usize| u8::try_from(self.parser.param(at + i)).unwrap_or(255);
                let index = nearest_index(channel(1), channel(2), channel(3));
                self.set_color(foreground, u16::from(index));
                4
            }
            _ => available,
        }
    }

    // ---- respostas ----

    fn push_reply(&mut self, bytes: &[u8]) {
        if self.reply_len + bytes.len() <= REPLY_CAP {
            self.reply[self.reply_len..self.reply_len + bytes.len()].copy_from_slice(bytes);
            self.reply_len += bytes.len();
        }
    }

    fn device_status(&mut self) {
        match self.parser.param(0) {
            5 => self.push_reply(b"\x1b[0n"),
            6 => {
                let row = if self.origin() {
                    self.row.saturating_sub(self.scroll_top)
                } else {
                    self.row
                };
                let mut buf = [0u8; 48];
                buf[..2].copy_from_slice(b"\x1b[");
                let mut pos = put_number(&mut buf, 2, row + 1);
                buf[pos] = b';';
                pos = put_number(&mut buf, pos + 1, self.col + 1);
                buf[pos] = b'R';
                self.push_reply(&buf[..=pos]);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
