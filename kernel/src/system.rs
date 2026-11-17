//! O kernel visto pelo shell: implementa a porta `Host` com os números reais
//! de memória, boot e tela, e roda o laço de entrada.

use rustos_hal::{BootInfo, Cpu, PAGE_SIZE, RegionKind};
use rustos_shell::{DisplayInfo, FaultTest, Host, MemoryInfo, Shell, SystemInfo};
use rustos_term::Layout;

use crate::console::{self, Sink};
use crate::heap;
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

/// Tiques que o autoteste espera: 50 ms a 100 Hz. Fora de 25–200 ms, a calibração
/// errou; sem tique em 1 s, o timer não chega.
const SELF_TEST_MS: core::ops::RangeInclusive<u64> = 25..=200;
const SELF_TEST_GIVE_UP_NS: u64 = 1_000_000_000;
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
        let frames = self.memory.frames;
        let heap = heap::stats();
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
                frame_metadata_bytes: self.memory.metadata_bytes,
                heap_pages: heap.pages,
                heap_used_bytes: heap.used_bytes,
                kernel_bytes: self.region_bytes(&[RegionKind::KernelAndModules]),
            },
            display: self.display,
        }
    }

    fn uptime_ns(&self) -> Option<u64> {
        platform::now_ns()
    }

    fn keymap(&self) -> Option<&'static str> {
        let usable = platform::has_keyboard() && console::has_screen();
        usable.then(|| KEYMAP.get()).flatten()
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
    let elapsed = || platform::now_ns().unwrap_or(0);
    let (start_ticks, start_ns) = (interrupts::ticks(), elapsed());
    while interrupts::ticks() < start_ticks + SELF_TEST_TICKS
        && elapsed().saturating_sub(start_ns) < SELF_TEST_GIVE_UP_NS
    {
        cpu.save_and_disable_interrupts();
        cpu.enable_interrupts_and_wait();
    }
    let elapsed_ms = elapsed().saturating_sub(start_ns) / 1_000_000;
    let ticked = interrupts::ticks() >= start_ticks + SELF_TEST_TICKS;
    let timer_ok = ticked && SELF_TEST_MS.contains(&elapsed_ms);
    kprintln!(
        "[rustos] timer: {} Hz, {SELF_TEST_TICKS} ticks in {elapsed_ms} ms {}",
        platform::TIMER_HZ,
        if timer_ok { "ok" } else { "FAILED" }
    );
    breakpoint_ok && timer_ok
}

/// Laço do shell: tira bytes da fila que as IRQs de serial e teclado enchem, e
/// dorme até a próxima interrupção quando ela está vazia.
pub fn run_shell(host: &mut impl Host) -> ! {
    let mut shell = Shell::<LINE_BYTES, HISTORY_LINES>::new();
    shell.start(host, &mut Sink).ok();
    console::flush();
    let cpu = platform::cpu();
    loop {
        let were_enabled = cpu.save_and_disable_interrupts();
        let Some(byte) = interrupts::pop_input() else {
            cpu.enable_interrupts_and_wait();
            continue;
        };
        cpu.restore_interrupts(were_enabled);
        shell.feed(byte, host, &mut Sink).ok();
        console::flush();
    }
}
