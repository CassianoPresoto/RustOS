//! `hal::RegisterBus` sobre as instruções `in`/`out` de E/S por porta.

use core::arch::asm;

use rustos_hal::RegisterBus;

#[derive(Clone, Copy, Debug)]
pub struct PortBus {
    base: u16,
}

impl PortBus {
    /// # Safety
    /// As portas a partir de `base` pertencem a um dispositivo que tolera os
    /// acessos do driver, e nenhum outro código as usa ao mesmo tempo.
    pub const unsafe fn new(base: u16) -> Self {
        Self { base }
    }

    fn port(self, offset: usize) -> u16 {
        let offset = u16::try_from(offset).expect("deslocamento de porta cabe em 16 bits");
        self.base.wrapping_add(offset)
    }
}

impl RegisterBus for PortBus {
    fn read8(&self, offset: usize) -> u8 {
        let value: u8;
        // SAFETY: a porta é do dispositivo, pelo contrato de `new`.
        unsafe {
            asm!("in al, dx", out("al") value, in("dx") self.port(offset),
                 options(nostack, preserves_flags));
        }
        value
    }

    fn write8(&self, offset: usize, value: u8) {
        // SAFETY: a porta é do dispositivo, pelo contrato de `new`.
        unsafe {
            asm!("out dx, al", in("dx") self.port(offset), in("al") value,
                 options(nostack, preserves_flags));
        }
    }

    fn read32(&self, offset: usize) -> u32 {
        let value: u32;
        // SAFETY: a porta é do dispositivo, pelo contrato de `new`.
        unsafe {
            asm!("in eax, dx", out("eax") value, in("dx") self.port(offset),
                 options(nostack, preserves_flags));
        }
        value
    }

    fn write32(&self, offset: usize, value: u32) {
        // SAFETY: a porta é do dispositivo, pelo contrato de `new`.
        unsafe {
            asm!("out dx, eax", in("dx") self.port(offset), in("eax") value,
                 options(nostack, preserves_flags));
        }
    }
}
