//! I/O APIC: registradores indiretos (`IOREGSEL`/`IOWIN`) e a tabela de
//! redirecionamento, sobre um `RegisterBus`. Puro.

use rustos_hal::RegisterBus;

/// Endereço fixo do I/O APIC na QEMU `q35` e na maioria dos PCs (DEBT-0009).
pub const IOAPIC_BASE: u64 = 0xFEC0_0000;

const IOREGSEL: usize = 0x00;
const IOWIN: usize = 0x10;
const REG_VERSION: u32 = 0x01;
const REG_REDIRECTION: u32 = 0x10;

/// Máscara de uma entrada: bit 16 do dword baixo.
const MASKED: u32 = 1 << 16;
/// Destino físico (bit 11 = 0), polaridade alta (13), borda (15), entrega fixa (10:8 = 0).
const DESTINATION_SHIFT: u32 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PinOutOfRange;

/// As duas metades de uma entrada de redirecionamento.
pub const fn route_entry(vector: u8, destination: u8, masked: bool) -> (u32, u32) {
    let low = vector as u32 | if masked { MASKED } else { 0 };
    (low, (destination as u32) << DESTINATION_SHIFT)
}

#[derive(Debug)]
pub struct IoApic<B> {
    bus: B,
}

impl<B: RegisterBus> IoApic<B> {
    pub const fn new(bus: B) -> Self {
        Self { bus }
    }

    fn read(&self, register: u32) -> u32 {
        self.bus.write32(IOREGSEL, register);
        self.bus.read32(IOWIN)
    }

    fn write(&self, register: u32, value: u32) {
        self.bus.write32(IOREGSEL, register);
        self.bus.write32(IOWIN, value);
    }

    /// Número de entradas de redirecionamento (campo MRE da versão, mais um).
    pub fn pins(&self) -> u32 {
        ((self.read(REG_VERSION) >> 16) & 0xFF) + 1
    }

    /// Roteia o pino para `vector` no APIC local `destination`. O alto vai primeiro:
    /// a entrada só passa a valer quando o dword baixo é escrito.
    pub fn route(
        &self,
        pin: u8,
        vector: u8,
        destination: u8,
        masked: bool,
    ) -> Result<(), PinOutOfRange> {
        if u32::from(pin) >= self.pins() {
            return Err(PinOutOfRange);
        }
        let (low, high) = route_entry(vector, destination, masked);
        let register = REG_REDIRECTION + 2 * u32::from(pin);
        self.write(register + 1, high);
        self.write(register, low);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use core::cell::{Cell, RefCell};

    use super::*;

    #[derive(Default)]
    struct FakeBus {
        selected: Cell<u32>,
        writes: RefCell<Vec<(u32, u32)>>,
        max_entry: u32,
    }

    impl RegisterBus for FakeBus {
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }

        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }

        fn read32(&self, offset: usize) -> u32 {
            assert_eq!(offset, IOWIN);
            assert_eq!(self.selected.get(), REG_VERSION);
            self.max_entry << 16 | 0x20
        }

        fn write32(&self, offset: usize, value: u32) {
            if offset == IOREGSEL {
                self.selected.set(value);
            } else {
                assert_eq!(offset, IOWIN);
                self.writes.borrow_mut().push((self.selected.get(), value));
            }
        }
    }

    fn ioapic() -> IoApic<FakeBus> {
        IoApic::new(FakeBus {
            max_entry: 23,
            ..FakeBus::default()
        })
    }

    #[test]
    fn counts_pins_from_the_version_register() {
        assert_eq!(ioapic().pins(), 24);
    }

    #[test]
    fn route_writes_destination_before_the_vector() {
        let io = ioapic();
        io.route(4, 0x24, 0, false).unwrap();
        assert_eq!(*io.bus.writes.borrow(), [(0x19, 0), (0x18, 0x24)]);
    }

    #[test]
    fn masked_route_sets_bit_16_and_keeps_the_destination() {
        let io = ioapic();
        io.route(1, 0x21, 5, true).unwrap();
        assert_eq!(
            *io.bus.writes.borrow(),
            [(0x13, 5 << 24), (0x12, 0x21 | 1 << 16)]
        );
    }

    #[test]
    fn pin_beyond_the_table_is_rejected() {
        let io = ioapic();
        assert_eq!(io.route(24, 0x30, 0, false), Err(PinOutOfRange));
        assert!(io.bus.writes.borrow().is_empty());
    }

    #[test]
    fn entry_is_fixed_physical_active_high_edge() {
        let (low, _) = route_entry(0x24, 0, false);
        assert_eq!(low & !0xFF, 0);
    }
}
