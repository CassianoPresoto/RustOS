//! QEMU `virt` aarch64: PL011 por MMIO, mapeada como dispositivo pelo kernel.

use rustos_arch_aarch64::{Aarch64Cpu, Aarch64PageMapper, MmioBus, QEMU_VIRT_PL011};
use rustos_driver_pl011::Pl011;
use rustos_hal::{BootInfo, FrameSource, MapError, PAGE_SIZE, PhysAddr};

pub const ARCH: &str = "aarch64";

pub type PlatformConsole = Pl011<MmioBus>;

pub fn cpu() -> Aarch64Cpu {
    // SAFETY: o Limine entrega o kernel em EL1 (ou EL2 com VHE), nunca em EL0.
    unsafe { Aarch64Cpu::new() }
}

pub fn console(
    boot: &impl BootInfo,
    frames: &mut impl FrameSource,
) -> Result<PlatformConsole, MapError> {
    // SAFETY: MMU ligada; as tabelas do Limine e os frames novos estão no mapa direto.
    let mut mapper = unsafe { Aarch64PageMapper::active(boot.direct_map_offset()) };
    let registers = PhysAddr::new(QEMU_VIRT_PL011);
    let virt = crate::memory::map_device(&mut mapper, frames, registers, PAGE_SIZE)?;
    // SAFETY: a página acabou de ser mapeada como dispositivo e cobre todo o bloco da PL011.
    let bus = unsafe { MmioBus::new(virt) };
    let mut uart = Pl011::new(bus);
    uart.init();
    Ok(uart)
}

/// Frequência do tique do timer.
pub const TIMER_HZ: u32 = 100;

/// Liga exceções, controlador de interrupção, timer em `TIMER_HZ` e a IRQ de
/// recepção da serial. Deixa a máscara da CPU desligada: o chamador decide.
pub fn init_interrupts(
    _boot: &impl rustos_hal::BootInfo,
    _memory: &mut crate::memory::Memory,
    _handlers: &'static rustos_hal::Handlers,
) -> Result<(), super::PlatformError> {
    Err(super::PlatformError::Unsupported)
}

/// Traduz o número da IRQ para a fonte que o núcleo entende.
pub fn irq_source(_irq: rustos_hal::Irq) -> crate::interrupts::IrqSource {
    crate::interrupts::IrqSource::Unknown
}

/// Chamado a cada tique, dentro da IRQ: rearma o timer se ele for one-shot.
pub fn timer_tick() {}

/// Chamado depois de drenar a UART, dentro da IRQ: limpa a interrupção dela.
pub fn serial_irq_done() {}

/// A `virt` não tem teclado nesta fase: a entrada é a serial.
pub fn has_keyboard() -> bool {
    false
}

/// Próxima tecla do teclado da plataforma, dentro da IRQ; `None` se não há.
pub fn keyboard_event() -> Option<rustos_hal::KeyEvent> {
    None
}

/// Nanossegundos desde que `init_interrupts` ligou o relógio; `None` antes.
pub fn now_ns() -> Option<u64> {
    None
}

/// As proteções de hardware desta ISA e se ficaram ligadas.
pub fn protections() -> &'static [super::Protection] {
    &[]
}

/// Provoca a exceção pedida; `PageFault` não volta.
pub fn trigger(_test: crate::interrupts::FaultTest) -> bool {
    false
}
