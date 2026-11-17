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
