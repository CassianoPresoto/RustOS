//! PC x86_64: COM1 por porta de E/S, sem precisar de mapeamento.

use rustos_arch_x86_64::{COM1, PortBus, X86Cpu};
use rustos_driver_uart16550::{DIVISOR_115200, Uart16550};
use rustos_hal::{BootInfo, FrameSource, MapError};

pub const ARCH: &str = "x86_64";

pub type PlatformConsole = Uart16550<PortBus>;

pub fn cpu() -> X86Cpu {
    // SAFETY: o Limine entrega o kernel em ring 0, e ele nunca sai de lá.
    unsafe { X86Cpu::new() }
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "mesma assinatura em todas as ISAs"
)]
pub fn console(
    _boot: &impl BootInfo,
    _frames: &mut impl FrameSource,
) -> Result<PlatformConsole, MapError> {
    // SAFETY: a COM1 é a UART padrão do PC e só o console a usa.
    let bus = unsafe { PortBus::new(COM1) };
    let mut uart = Uart16550::new(bus);
    uart.init(DIVISOR_115200);
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
