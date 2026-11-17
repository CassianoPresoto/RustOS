//! Barramento de registradores de um dispositivo: E/S por porta (x86) ou MMIO.

/// O deslocamento é relativo à base do dispositivo. Quem constrói a
/// implementação garante que a base aponta para o dispositivo certo.
pub trait RegisterBus {
    fn read8(&self, offset: usize) -> u8;
    fn write8(&self, offset: usize, value: u8);
    fn read32(&self, offset: usize) -> u32;
    fn write32(&self, offset: usize, value: u32);
}
