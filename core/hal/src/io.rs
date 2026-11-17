//! Barramento de registradores de um dispositivo: E/S por porta (x86) ou MMIO.

/// O deslocamento é relativo à base do dispositivo. Quem constrói a
/// implementação garante que a base aponta para o dispositivo certo.
pub trait RegisterBus {
    fn read8(&self, offset: usize) -> u8;
    fn write8(&self, offset: usize, value: u8);
    fn read32(&self, offset: usize) -> u32;
    fn write32(&self, offset: usize, value: u32);

    /// Registrador de 64 bits. Sem acesso nativo, são dois de 32 bits, o baixo primeiro.
    fn read64(&self, offset: usize) -> u64 {
        u64::from(self.read32(offset)) | u64::from(self.read32(offset + 4)) << 32
    }

    /// Ver `read64`.
    fn write64(&self, offset: usize, value: u64) {
        #[allow(clippy::cast_possible_truncation, reason = "metade baixa de propósito")]
        self.write32(offset, value as u32);
        self.write32(offset + 4, (value >> 32) as u32);
    }
}

#[cfg(test)]
mod tests {
    use core::cell::RefCell;

    use super::*;

    #[derive(Default)]
    struct Narrow(RefCell<[u32; 2]>);

    impl RegisterBus for Narrow {
        fn read8(&self, _: usize) -> u8 {
            0
        }
        fn write8(&self, _: usize, _: u8) {}
        fn read32(&self, offset: usize) -> u32 {
            self.0.borrow()[offset / 4]
        }
        fn write32(&self, offset: usize, value: u32) {
            self.0.borrow_mut()[offset / 4] = value;
        }
    }

    #[test]
    fn default_64_bit_access_splits_into_two_32_bit_words() {
        let bus = Narrow::default();
        bus.write64(0, 0x1122_3344_5566_7788);
        assert_eq!(*bus.0.borrow(), [0x5566_7788, 0x1122_3344]);
        assert_eq!(bus.read64(0), 0x1122_3344_5566_7788);
    }
}
