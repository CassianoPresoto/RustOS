//! Apoio dos testes: `Host` falso e uma tela VT mínima para conferir o redesenho.

use core::fmt;

use rustos_hal::{BootTimeline, BootloaderId};

use crate::{
    BenchReport, CpuInfo, DisplayInfo, Host, KillOutcome, Listing, MemoryInfo, MicroBench,
    ProgramInfo, RunOutcome, ScaleSample, StormSample, SystemInfo, TaskInfo, TaskState,
};

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
    pub(crate) random: Option<u64>,
    pub(crate) bench: Option<BenchReport>,
    pub(crate) micro: Option<MicroBench>,
    pub(crate) alternate: bool,
    pub(crate) listing: Listing,
    /// O que `run_program` responde, e o que ele recebeu da última chamada.
    pub(crate) outcome: RunOutcome,
    pub(crate) last_run: core::cell::RefCell<Option<(String, Vec<String>, bool)>>,
    /// O que `kill_program` responde, e o pid que ele recebeu da última chamada.
    pub(crate) kill: KillOutcome,
    pub(crate) last_kill: core::cell::Cell<Option<u32>>,
}

impl FakeHost {
    pub(crate) fn new() -> Self {
        Self {
            info: info(),
            uptime: Some(65_000_000_000),
            size: (80, 24),
            faults: false,
            keymap: core::cell::Cell::new(Some("us")),
            random: Some(0x0123_4567_89ab_cdef),
            alternate: true,
            listing: Listing::Done,
            outcome: RunOutcome::Exited { pid: 4, code: 0 },
            last_run: core::cell::RefCell::new(None),
            kill: KillOutcome::Killed,
            last_kill: core::cell::Cell::new(None),
            micro: Some(MicroBench {
                zero_kib_tenths_ns: 153,
                zero_page_tenths_ns: 205,
                zero_cold_tenths_ns: 206,
                zero_pair_tenths_ns: 566,
                random_mb_per_s: 640,
                random_reseed_ns: 38_000,
                random_u64_tenths_ns: 963,
            }),
            bench: Some(BenchReport {
                storm: StormSample {
                    cpus: 4,
                    iterations: 20_000,
                    locked_ns: 90_000_000,
                    cached_ns: 9_000_000,
                    locked_contention: 5_000,
                    cached_contention: 40,
                    heap_ns: 50_000_000,
                    heap_contention: 3_000,
                },
                switch_ns: 412,
                switch_simd_ns: 530,
                wake_samples: 200,
                wake_mean_ns: 12_345,
                wake_max_ns: 45_678,
                wake_loaded_mean_ns: 20_000,
                wake_loaded_max_ns: 99_999,
                wake_task_median_ns: 84,
                wake_task_mean_ns: 97,
                wake_task_max_ns: 250,
                pick_next_ns: 88,
                tick_ns: 31,
                switch_cross_ns: 3_210,
                ipi_rtt_ns: 4_560,
                cpus: 4,
                scale: [
                    ScaleSample {
                        cpus: 1,
                        tasks: 8,
                        total_ns: 80_000_000,
                    },
                    ScaleSample {
                        cpus: 2,
                        tasks: 8,
                        total_ns: 40_000_000,
                    },
                    ScaleSample {
                        cpus: 4,
                        tasks: 8,
                        total_ns: 20_500_000,
                    },
                ],
            }),
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

    fn has_alternate_screen(&self) -> bool {
        self.alternate
    }

    fn fault_test(&self, _kind: crate::host::FaultTest) -> bool {
        self.faults
    }

    fn random_u64(&self) -> Option<u64> {
        self.random
    }

    fn tasks(&self, visit: &mut dyn FnMut(&TaskInfo<'_>)) {
        let task = |id, name, state, cpu_ns, simd| TaskInfo {
            id,
            name,
            pid: (name == "count").then_some(id + 40),
            state,
            cpu: u8::try_from(id).unwrap(),
            cpu_ns,
            switches: 7,
            simd,
            stack_peak: 3_500,
            stack_bytes: 16_384,
        };
        visit(&task(0, "idle", TaskState::Ready, 250_000_000, false));
        visit(&task(1, "shell", TaskState::Running, 12_345_678_901, false));
        visit(&task(2, "worker", TaskState::Sleeping, 1_000_000, true));
        visit(&task(3, "count", TaskState::Blocked, 2_000_000, true));
    }

    fn cpus(&self, visit: &mut dyn FnMut(&CpuInfo)) {
        for index in 0..2 {
            visit(&CpuInfo {
                index,
                load: 2 - u32::from(index),
                idle_ns: 32_500_000_000 * u64::from(index),
                switches: 1_000 + u64::from(index),
                ipis_sent: 7,
                ipis_received: 9,
                pulled: 3,
            });
        }
    }

    fn lock_contention(&self, visit: &mut dyn FnMut(&str, u64)) {
        visit("heap", 12);
        visit("frames", 3);
    }

    fn bench(&self) -> Option<BenchReport> {
        self.bench
    }

    fn programs(&self, visit: &mut dyn FnMut(&ProgramInfo<'_>)) -> Listing {
        if self.listing == Listing::Done {
            visit(&ProgramInfo {
                name: "init",
                bytes: 11_584,
            });
            visit(&ProgramInfo {
                name: "hello",
                bytes: 4_096,
            });
        }
        self.listing
    }

    fn run_program(&self, name: &str, args: &[&str], wait: bool) -> RunOutcome {
        let args = args.iter().map(|arg| (*arg).to_owned()).collect();
        *self.last_run.borrow_mut() = Some((name.to_owned(), args, wait));
        self.outcome
    }

    fn kill_program(&self, pid: u32) -> KillOutcome {
        self.last_kill.set(Some(pid));
        self.kill
    }

    fn micro_bench(&self) -> Option<MicroBench> {
        self.micro
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
    /// `\n` também volta à coluna 0, como o `onlcr` do console do kernel.
    onlcr: bool,
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
            onlcr: false,
            parse: Parse::Ground,
            params: String::new(),
            raw: String::new(),
            bells: 0,
        }
    }

    pub(crate) fn with_onlcr(mut self) -> Self {
        self.onlcr = true;
        self
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
            (Parse::Ground, '\n') => {
                self.newline();
                if self.onlcr {
                    self.x = 0;
                }
            }
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
