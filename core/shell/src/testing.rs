//! Apoio dos testes: `Host` falso e uma tela VT mínima para conferir o redesenho.

use core::fmt;

use rustos_hal::{BootTimeline, BootloaderId};

use crate::{DisplayInfo, Host, MemoryInfo, SystemInfo};

pub(crate) fn info() -> SystemInfo<'static> {
    SystemInfo {
        os_name: "RustOS",
        os_version: "0.1.0",
        arch: "x86_64",
        bootloader: Some(BootloaderId {
            name: "Limine",
            version: "9.2.3",
        }),
        boot_timeline: Some(BootTimeline {
            reset_us: 0,
            loader_start_us: 312_400,
            kernel_handoff_us: 320_500,
        }),
        cpu: Some("Test CPU @ 3.0GHz"),
        memory: MemoryInfo {
            usable_bytes: 512 << 20,
            free_bytes: 500 << 20,
            reserved_bytes: 1_572_864,
            frame_metadata_bytes: 16_384,
            heap_pages: 12,
            heap_used_bytes: 5_000,
            kernel_bytes: 2_306_868,
        },
        display: Some(DisplayInfo {
            width: 1280,
            height: 800,
            cols: 160,
            rows: 50,
        }),
    }
}

pub(crate) struct FakeHost {
    pub(crate) info: SystemInfo<'static>,
    pub(crate) uptime: Option<u64>,
    pub(crate) size: (u16, u16),
    pub(crate) faults: bool,
    pub(crate) keymap: core::cell::Cell<Option<&'static str>>,
}

impl FakeHost {
    pub(crate) fn new() -> Self {
        Self {
            info: info(),
            uptime: Some(65_000_000_000),
            size: (80, 24),
            faults: false,
            keymap: core::cell::Cell::new(Some("us")),
        }
    }
}

impl Host for FakeHost {
    fn info(&self) -> SystemInfo<'_> {
        self.info
    }

    fn uptime_ns(&self) -> Option<u64> {
        self.uptime
    }

    fn terminal_size(&self) -> (u16, u16) {
        self.size
    }

    fn fault_test(&self, _kind: crate::host::FaultTest) -> bool {
        self.faults
    }

    fn keymap(&self) -> Option<&'static str> {
        self.keymap.get()
    }

    fn set_keymap(&self, name: &str) -> bool {
        let known = match name {
            "us" => Some("us"),
            "br" => Some("br"),
            _ => None,
        };
        if known.is_some() && self.keymap.get().is_some() {
            self.keymap.set(known);
        }
        known.is_some()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Parse {
    Ground,
    Escape,
    Csi,
}

/// Terminal VT mínimo: texto, CR, LF, BS, CUU/CUD/CUF/CUB, EL, ED e CUP.
/// `deferred` escolhe a regra da última coluna: xterm (quebra adiada) ou quebra imediata.
pub(crate) struct Screen {
    cols: usize,
    rows: usize,
    cells: Vec<Vec<char>>,
    x: usize,
    y: usize,
    pending: bool,
    deferred: bool,
    parse: Parse,
    params: String,
    pub(crate) raw: String,
    pub(crate) bells: usize,
}

impl Screen {
    pub(crate) fn new(cols: usize, rows: usize, deferred: bool) -> Self {
        Self {
            cols,
            rows,
            cells: vec![vec![' '; cols]; rows],
            x: 0,
            y: 0,
            pending: false,
            deferred,
            parse: Parse::Ground,
            params: String::new(),
            raw: String::new(),
            bells: 0,
        }
    }

    pub(crate) fn cursor(&self) -> (usize, usize) {
        (self.y, self.x)
    }

    pub(crate) fn row(&self, y: usize) -> String {
        self.cells[y]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// Todas as linhas, sem as vazias do fim.
    pub(crate) fn text(&self) -> String {
        let rows: Vec<String> = (0..self.rows).map(|y| self.row(y)).collect();
        rows.join("\n").trim_end().to_string()
    }

    fn newline(&mut self) {
        self.pending = false;
        if self.y + 1 == self.rows {
            self.cells.remove(0);
            self.cells.push(vec![' '; self.cols]);
        } else {
            self.y += 1;
        }
    }

    fn put(&mut self, c: char) {
        if self.pending {
            self.x = 0;
            self.newline();
        }
        self.cells[self.y][self.x] = c;
        if self.x + 1 < self.cols {
            self.x += 1;
        } else if self.deferred {
            self.pending = true;
        } else {
            self.x = 0;
            self.newline();
        }
    }

    fn csi(&mut self, final_byte: char) {
        let n = self
            .params
            .split(';')
            .next()
            .and_then(|p| p.parse::<usize>().ok());
        let step = n.unwrap_or(1).max(1);
        self.pending = false;
        match final_byte {
            'A' => self.y = self.y.saturating_sub(step),
            'B' => self.y = (self.y + step).min(self.rows - 1),
            'C' => self.x = (self.x + step).min(self.cols - 1),
            'D' => self.x = self.x.saturating_sub(step),
            'H' => (self.x, self.y) = (0, 0),
            'K' => self.cells[self.y][self.x..].fill(' '),
            'J' => match n.unwrap_or(0) {
                0 => {
                    self.cells[self.y][self.x..].fill(' ');
                    self.cells[self.y + 1..]
                        .iter_mut()
                        .for_each(|row| row.fill(' '));
                }
                2 => self.cells.iter_mut().for_each(|row| row.fill(' ')),
                _ => {}
            },
            _ => {}
        }
    }

    fn feed(&mut self, c: char) {
        match (self.parse, c) {
            (Parse::Ground, '\x1b') => self.parse = Parse::Escape,
            (Parse::Ground, '\r') => (self.x, self.pending) = (0, false),
            (Parse::Ground, '\n') => self.newline(),
            (Parse::Ground, '\x08') => self.x = self.x.saturating_sub(1),
            (Parse::Ground, '\x07') => self.bells += 1,
            (Parse::Ground, c) if c >= ' ' => self.put(c),
            (Parse::Escape, '[') => (self.parse, self.params) = (Parse::Csi, String::new()),
            (Parse::Escape, _) => self.parse = Parse::Ground,
            (Parse::Csi, c) if c.is_ascii_digit() || c == ';' || c == '?' => self.params.push(c),
            (Parse::Csi, c) => {
                self.csi(c);
                self.parse = Parse::Ground;
            }
            _ => {}
        }
    }
}

impl fmt::Write for Screen {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.raw.push_str(s);
        s.chars().for_each(|c| self.feed(c));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write;

    #[test]
    fn deferred_wrap_waits_for_the_next_char() {
        let mut screen = Screen::new(4, 3, true);
        screen.write_str("abcd").unwrap();
        assert_eq!(screen.cursor(), (0, 3));
        screen.write_str("e").unwrap();
        assert_eq!((screen.row(0), screen.row(1)), ("abcd".into(), "e".into()));
    }

    #[test]
    fn immediate_wrap_moves_at_the_last_column() {
        let mut screen = Screen::new(4, 3, false);
        screen.write_str("abcd").unwrap();
        assert_eq!(screen.cursor(), (1, 0));
    }

    #[test]
    fn newline_at_the_bottom_scrolls() {
        let mut screen = Screen::new(4, 2, true);
        screen.write_str("a\r\nb\r\nc").unwrap();
        assert_eq!(screen.text(), "b\nc");
    }

    #[test]
    fn cursor_moves_are_clamped_and_erase_works() {
        let mut screen = Screen::new(4, 3, true);
        screen
            .write_str("abcd\r\nefgh\x1b[5A\r\x1b[2C\x1b[K\x1b[B\x1b[J")
            .unwrap();
        assert_eq!(screen.row(0), "ab");
        assert_eq!(screen.row(1), "ef");
        assert_eq!(screen.cursor(), (1, 2));
    }
}
