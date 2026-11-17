//! O kernel visto pelo shell: implementa a porta `Host` com os números reais
//! de memória, boot e tela, e roda o laço de entrada.

use core::hint::spin_loop;

use rustos_hal::{BootInfo, PAGE_SIZE, RegionKind};
use rustos_shell::{DisplayInfo, Host, MemoryInfo, Shell, SystemInfo};

use crate::console::{self, Sink};
use crate::memory::Memory;

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
        None
    }

    fn terminal_size(&self) -> (u16, u16) {
        self.display
            .map_or(SERIAL_SIZE, |display| (display.cols, display.rows))
    }
}

/// Laço do shell. A entrada é por consulta até existir interrupção (DEBT-0007).
pub fn run_shell(host: &mut impl Host) -> ! {
    let mut shell = Shell::<LINE_BYTES, HISTORY_LINES>::new();
    shell.start(host, &mut Sink).ok();
    console::flush();
    loop {
        match console::read_byte() {
            Some(byte) => {
                shell.feed(byte, host, &mut Sink).ok();
                console::flush();
            }
            None => spin_loop(),
        }
    }
}
