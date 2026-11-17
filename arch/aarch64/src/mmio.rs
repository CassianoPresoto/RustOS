//! `hal::RegisterBus` por acesso volátil a memória mapeada como dispositivo.

use core::ptr;

use rustos_hal::{RegisterBus, VirtAddr};

#[derive(Clone, Copy, Debug)]
pub struct MmioBus {
    base: usize,
}

impl MmioBus {
    /// # Safety
    /// `base` é o endereço virtual de um bloco de registradores mapeado como
    /// dispositivo, que cobre todo deslocamento que o driver usar.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "aarch64 tem usize de 64 bits"
    )]
    pub const unsafe fn new(base: VirtAddr) -> Self {
        Self {
            base: base.as_u64() as usize,
        }
    }

    fn register<T>(&self, offset: usize) -> *mut T {
        debug_assert!((self.base + offset).is_multiple_of(align_of::<T>()));
        ptr::with_exposed_provenance_mut(self.base + offset)
    }
}

impl RegisterBus for MmioBus {
    fn read8(&self, offset: usize) -> u8 {
        // SAFETY: o registrador está no bloco mapeado, pelo contrato de `new`.
        unsafe { self.register::<u8>(offset).read_volatile() }
    }

    fn write8(&self, offset: usize, value: u8) {
        // SAFETY: o registrador está no bloco mapeado, pelo contrato de `new`.
        unsafe { self.register::<u8>(offset).write_volatile(value) }
    }

    fn read32(&self, offset: usize) -> u32 {
        // SAFETY: o registrador está no bloco mapeado e alinhado, pelo contrato de `new`.
        unsafe { self.register::<u32>(offset).read_volatile() }
    }

    fn write32(&self, offset: usize, value: u32) {
        // SAFETY: o registrador está no bloco mapeado e alinhado, pelo contrato de `new`.
        unsafe { self.register::<u32>(offset).write_volatile(value) }
    }
}
