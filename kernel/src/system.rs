//! O kernel visto pelo shell: implementa a porta `Host` com os números reais
//! de memória, boot e tela, e roda o laço de entrada.

use core::hint::spin_loop;

use rustos_hal::{BootInfo, Cpu, PAGE_SIZE, RegionKind};
use rustos_shell::{DisplayInfo, FaultTest, Host, MemoryInfo, Shell, SystemInfo};
use rustos_term::Layout;

use crate::console::{self, Sink};
use crate::interrupts;
use crate::memory::Memory;
use crate::platform;

/// Layout em uso; só o laço do shell lê e escreve, numa CPU.
static KEYMAP: KeymapCell = KeymapCell(core::cell::Cell::new(Some("us")));

struct KeymapCell(core::cell::Cell<Option<&'static str>>);

// SAFETY: só o laço do shell, numa CPU e fora de interrupção, usa a célula.
unsafe impl Sync for KeymapCell {}

impl KeymapCell {
    fn get(&self) -> Option<&'static str> {
        self.0.get()
    }

    fn set(&self, value: Option<&'static str>) {
        self.0.set(value);
    }
}

/// Tiques que o autoteste espera: 50 ms a 100 Hz.
const SELF_TEST_TICKS: u64 = 5;

/// Tamanho que a serial assume quando não há tela.
const SERIAL_SIZE: (u16, u16) = (80, 24);
const LINE_BYTES: usize = 256;
const HISTORY_LINES: usize = 32;

#[derive(Debug)]
pub struct System<'a, B> {
    boot: &'a B,
    memory: &'a Memory,
    display: Option<DisplayInfo>,
}

impl<'a, B: BootInfo> System<'a, B> {
    pub fn new(boot: &'a B, memory: &'a Memory, display: Option<DisplayInfo>) -> Self {
        Self {
            boot,
            memory,
            display,
        }
    }

    /// Bytes em RAM que o sistema ocupa ou que o firmware guardou, fora o livre.
    fn region_bytes(&self, kinds: &[RegionKind]) -> u64 {
        self.boot
            .memory_regions()
            .filter(|region| kinds.contains(&region.kind))
            .map(|region| region.len)
            .sum()
    }
}

impl<B: BootInfo> Host for System<'_, B> {
    fn info(&self) -> SystemInfo<'_> {
        let frames = &self.memory.frames;
        SystemInfo {
            os_name: "RustOS",
            os_version: env!("CARGO_PKG_VERSION"),
            arch: crate::platform::ARCH,
            bootloader: self.boot.bootloader(),
            boot_timeline: self.boot.timeline(),
            cpu: None,
            memory: MemoryInfo {
                usable_bytes: self.memory.summary.usable_bytes,
                free_bytes: frames.free_frames() as u64 * PAGE_SIZE,
                reserved_bytes: self.region_bytes(&[
                    RegionKind::BootloaderReclaimable,
                    RegionKind::AcpiReclaimable,
                    RegionKind::AcpiNvs,
                ]),
                frame_bitmap_bytes: self.memory.bitmap_bytes,
                kernel_bytes: self.region_bytes(&[RegionKind::KernelAndModules]),
            },
            display: self.display,
        }
    }

    fn uptime_ns(&self) -> Option<u64> {
        platform::now_ns()
    }

    fn keymap(&self) -> Option<&'static str> {
        platform::has_keyboard().then(|| KEYMAP.get()).flatten()
    }

    fn set_keymap(&self, name: &str) -> bool {
        let (layout, label) = match name {
            "us" => (Layout::Us, "us"),
            "br" | "abnt2" => (Layout::BrAbnt2, "br"),
            _ => return false,
        };
        if console::set_layout(layout) {
            KEYMAP.set(Some(label));
        }
        true
    }

    fn fault_test(&self, kind: FaultTest) -> bool {
        let kind = match kind {
            FaultTest::Breakpoint => interrupts::FaultTest::Breakpoint,
            FaultTest::PageFault => interrupts::FaultTest::PageFault,
        };
        platform::trigger(kind)
    }

    fn terminal_size(&self) -> (u16, u16) {
        self.display
            .map_or(SERIAL_SIZE, |display| (display.cols, display.rows))
    }
}

/// Breakpoint que volta e tiques do timer contados contra o relógio.
pub fn interrupt_self_test() -> bool {
    let breakpoint_ok =
        platform::trigger(interrupts::FaultTest::Breakpoint) && interrupts::breakpoints() == 1;
    kprintln!(
        "[rustos] exception self-test: {}",
        if breakpoint_ok { "ok" } else { "FAILED" }
    );
    let cpu = platform::cpu();
    let (start_ticks, start_ns) = (interrupts::ticks(), platform::now_ns());
    while interrupts::ticks() < start_ticks + SELF_TEST_TICKS {
        cpu.save_and_disable_interrupts();
        cpu.enable_interrupts_and_wait();
    }
    let elapsed_ms = platform::now_ns()
        .zip(start_ns)
        .map_or(0, |(now, start)| now.saturating_sub(start) / 1_000_000);
    kprintln!(
        "[rustos] timer: {} Hz, {SELF_TEST_TICKS} ticks in {elapsed_ms} ms",
        platform::TIMER_HZ
    );
    breakpoint_ok
}

/// Laço do shell. Com interrupção, dorme até chegar tecla; sem, consulta a
/// serial em espera ativa (DEBT-0007).
pub fn run_shell(host: &mut impl Host, interrupts_on: bool) -> ! {
    let mut shell = Shell::<LINE_BYTES, HISTORY_LINES>::new();
    shell.start(host, &mut Sink).ok();
    console::flush();
    let cpu = platform::cpu();
    loop {
        let byte = if interrupts_on {
            let were_enabled = cpu.save_and_disable_interrupts();
            if let Some(byte) = interrupts::pop_input() {
                cpu.restore_interrupts(were_enabled);
                byte
            } else {
                cpu.enable_interrupts_and_wait();
                continue;
            }
        } else if let Some(byte) = console::read_byte() {
            byte
        } else {
            spin_loop();
            continue;
        };
        shell.feed(byte, host, &mut Sink).ok();
        console::flush();
    }
}
