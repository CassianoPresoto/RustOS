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
