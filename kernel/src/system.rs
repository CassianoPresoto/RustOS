//! O kernel visto pelo shell: implementa a porta `Host` com os números reais
//! de memória, boot e tela, e roda o laço de entrada.

use rustos_hal::{BootInfo, Cpu, PAGE_SIZE, RegionKind};
use rustos_shell::{
    BenchReport, CpuInfo, DisplayInfo, FaultTest, Host, MemoryInfo, Shell, SystemInfo, TaskInfo,
    TaskState,
};
use rustos_term::Layout;

use crate::bench;
use crate::console::{self, Sink};
use crate::heap;
use crate::interrupts;
use crate::memory::Memory;
use crate::platform;
use crate::random;
use crate::task::{self, State};

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

    fn random_u64(&self) -> Option<u64> {
        random::next_u64()
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
            FaultTest::WriteText => interrupts::FaultTest::WriteText,
            FaultTest::CpuPanic => return bench::panic_on_other_cpu(),
        };
        platform::trigger(kind)
    }

    fn has_alternate_screen(&self) -> bool {
        console::screen_modes().is_none_or(|(alt, _)| alt)
    }

    fn terminal_size(&self) -> (u16, u16) {
        self.display
            .map_or(SERIAL_SIZE, |display| (display.cols, display.rows))
    }

    fn tasks(&self, visit: &mut dyn FnMut(&TaskInfo<'_>)) {
        for snapshot in task::snapshot().into_iter().flatten() {
            let state = match snapshot.state {
                State::Running => TaskState::Running,
                State::Sleeping => TaskState::Sleeping,
                State::Blocked => TaskState::Blocked,
                State::Free | State::New | State::Ready | State::Exited => TaskState::Ready,
            };
            visit(&TaskInfo {
                id: snapshot.id,
                name: snapshot.name,
                state,
                cpu: u8::try_from(snapshot.cpu).unwrap_or(u8::MAX),
                cpu_ns: snapshot.cpu_ns,
                switches: snapshot.switches,
                simd: snapshot.simd,
            });
        }
    }

    fn cpus(&self, visit: &mut dyn FnMut(&CpuInfo)) {
        for snapshot in task::cpu_snapshots().into_iter().flatten() {
            visit(&CpuInfo {
                index: u8::try_from(snapshot.index).unwrap_or(u8::MAX),
                load: snapshot.load,
                idle_ns: snapshot.idle_ns,
                switches: snapshot.switches,
                ipis_sent: snapshot.ipis_sent,
                ipis_received: snapshot.ipis_received,
                pulled: snapshot.pulled,
            });
        }
    }

    fn lock_contention(&self, visit: &mut dyn FnMut(&str, u64)) {
        visit("heap", u64::from(heap::contentions()));
        visit("frames", u64::from(crate::frames::contentions()));
        visit("console", u64::from(console::contentions()));
        visit("run queues", u64::from(task::queue_contentions()));
    }

    fn bench(&self) -> Option<BenchReport> {
        bench::run(bench::FULL)
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

/// Laço do shell, a tarefa principal: tira bytes da fila que as IRQs de serial e
/// teclado enchem, e bloqueia (a tarefa, não a CPU) quando ela está vazia.
pub fn run_shell(host: &mut impl Host) -> ! {
    task::set_current_name("shell");
    let mut shell = Shell::<LINE_BYTES, HISTORY_LINES>::new();
    shell.start(host, &mut Sink).ok();
    console::flush();
    loop {
        let byte = interrupts::read_input_blocking();
        shell.feed(byte, host, &mut Sink).ok();
        console::flush();
    }
}
