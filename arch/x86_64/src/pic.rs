//! PIC 8259: só o que o kernel faz com ele, mascarar tudo para o APIC assumir. Puro.

use rustos_hal::RegisterBus;

/// Máscara de interrupções: porta 0x21 (mestre) e 0xA1 (escravo), a base mais 1.
pub const MASK_OFFSET: usize = 1;
const MASK_ALL: u8 = 0xFF;

/// `master` tem base 0x20 e `slave` base 0xA0.
pub fn mask_all(master: &impl RegisterBus, slave: &impl RegisterBus) {
    master.write8(MASK_OFFSET, MASK_ALL);
    slave.write8(MASK_OFFSET, MASK_ALL);
}

#[cfg(test)]
mod tests {
    use core::cell::RefCell;

    use super::*;

    #[derive(Default)]
    struct FakeBus(RefCell<Vec<(usize, u8)>>);

    impl RegisterBus for FakeBus {
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }

        fn write8(&self, offset: usize, value: u8) {
            self.0.borrow_mut().push((offset, value));
        }

        fn read32(&self, _: usize) -> u32 {
            unreachable!()
        }

        fn write32(&self, _: usize, _: u32) {
            unreachable!()
        }
    }

    #[test]
    fn masks_every_line_of_both_controllers() {
        let (master, slave) = (FakeBus::default(), FakeBus::default());
        mask_all(&master, &slave);
        assert_eq!(*master.0.borrow(), [(1, 0xFF)]);
        assert_eq!(*slave.0.borrow(), [(1, 0xFF)]);
    }
}
