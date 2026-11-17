//! UART PL011 da ARM, a serial da QEMU `virt` e de boa parte das placas aarch64.
//! Mantém o baud que o firmware programou: o relógio de referência varia por placa.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::hint::spin_loop;

use rustos_hal::{ByteSource, Console, RegisterBus};

const DATA: usize = 0x00;
const FLAG: usize = 0x18;
const LINE_CONTROL: usize = 0x2C;
const CONTROL: usize = 0x30;
const INTERRUPT_MASK: usize = 0x38;
const INTERRUPT_CLEAR: usize = 0x44;

const FLAG_BUSY: u32 = 1 << 3;
const FLAG_RX_EMPTY: u32 = 1 << 4;
const FLAG_TX_FULL: u32 = 1 << 5;
const DATA_BYTE: u32 = 0xFF;
const LINE_FIFO_ENABLE: u32 = 1 << 4;
const LINE_WORD_8BIT: u32 = 0b11 << 5;
const CONTROL_UART_ENABLE: u32 = 1 << 0;
const CONTROL_TX_ENABLE: u32 = 1 << 8;
const CONTROL_RX_ENABLE: u32 = 1 << 9;
const INTERRUPT_ALL: u32 = 0x7FF;

#[derive(Debug)]
pub struct Pl011<B> {
    bus: B,
}

impl<B: RegisterBus> Pl011<B> {
    pub const fn new(bus: B) -> Self {
        Self { bus }
    }

    /// 8N1 com FIFO, sem interrupções. Desliga a UART antes de mexer na linha, como manda o TRM.
    pub fn init(&mut self) {
        self.bus.write32(CONTROL, 0);
        while self.bus.read32(FLAG) & FLAG_BUSY != 0 {
            spin_loop();
        }
        self.bus.write32(INTERRUPT_MASK, 0);
        self.bus.write32(INTERRUPT_CLEAR, INTERRUPT_ALL);
        self.bus
            .write32(LINE_CONTROL, LINE_FIFO_ENABLE | LINE_WORD_8BIT);
        self.bus.write32(
            CONTROL,
            CONTROL_UART_ENABLE | CONTROL_TX_ENABLE | CONTROL_RX_ENABLE,
        );
    }

    pub fn write_byte(&mut self, byte: u8) {
        while self.bus.read32(FLAG) & FLAG_TX_FULL != 0 {
            spin_loop();
        }
        self.bus.write32(DATA, u32::from(byte));
    }
}

impl<B: RegisterBus> Console for Pl011<B> {
    fn write_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(byte);
        }
    }
}

impl<B: RegisterBus> ByteSource for Pl011<B> {
    /// Os bits de erro (8–11) do DATA são descartados: o byte chega como veio.
    fn read_byte(&mut self) -> Option<u8> {
        let empty = self.bus.read32(FLAG) & FLAG_RX_EMPTY != 0;
        (!empty).then(|| (self.bus.read32(DATA) & DATA_BYTE) as u8)
    }
}

#[cfg(test)]
mod tests {
    use core::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    use super::*;

    #[derive(Default)]
    struct FakeBus {
        writes: RefCell<Vec<(usize, u32)>>,
        full_polls: Cell<u32>,
        received: RefCell<VecDeque<u32>>,
    }

    impl RegisterBus for FakeBus {
        fn read8(&self, _: usize) -> u8 {
            unreachable!("PL011 só usa registradores de 32 bits")
        }

        fn write8(&self, _: usize, _: u8) {
            unreachable!("PL011 só usa registradores de 32 bits")
        }

        fn read32(&self, offset: usize) -> u32 {
            if offset == DATA {
                return self
                    .received
                    .borrow_mut()
                    .pop_front()
                    .expect("leu DATA sem dado");
            }
            assert_eq!(offset, FLAG);
            let empty = if self.received.borrow().is_empty() {
                FLAG_RX_EMPTY
            } else {
                0
            };
            if self.full_polls.get() > 0 {
                self.full_polls.set(self.full_polls.get() - 1);
                return FLAG_TX_FULL | empty;
            }
            empty
        }

        fn write32(&self, offset: usize, value: u32) {
            self.writes.borrow_mut().push((offset, value));
        }
    }

    #[test]
    fn init_disables_before_configuring_and_enables_last() {
        let mut uart = Pl011::new(FakeBus::default());
        uart.init();
        let writes = uart.bus.writes.borrow();
        assert_eq!(writes.first(), Some(&(CONTROL, 0)));
        assert_eq!(
            writes.last(),
            Some(&(
                CONTROL,
                CONTROL_UART_ENABLE | CONTROL_TX_ENABLE | CONTROL_RX_ENABLE
            ))
        );
    }

    #[test]
    fn waits_while_fifo_is_full() {
        let bus = FakeBus::default();
        bus.full_polls.set(2);
        let mut uart = Pl011::new(bus);
        uart.write_byte(b'z');
        assert_eq!(uart.bus.full_polls.get(), 0);
        assert_eq!(
            uart.bus.writes.borrow().as_slice(),
            &[(DATA, u32::from(b'z'))]
        );
    }

    #[test]
    fn newline_becomes_crlf() {
        let mut uart = Pl011::new(FakeBus::default());
        uart.write_bytes(b"\n");
        let data: Vec<u32> = uart.bus.writes.borrow().iter().map(|(_, v)| *v).collect();
        assert_eq!(data, [u32::from(b'\r'), u32::from(b'\n')]);
    }

    #[test]
    fn read_byte_is_none_while_rx_fifo_is_empty() {
        let mut uart = Pl011::new(FakeBus::default());
        assert_eq!(uart.read_byte(), None);
    }

    #[test]
    fn read_byte_drops_error_bits() {
        let bus = FakeBus::default();
        bus.received
            .borrow_mut()
            .extend([0x0F00 | u32::from(b'a'), u32::from(b'b')]);
        let mut uart = Pl011::new(bus);
        assert_eq!(uart.read_byte(), Some(b'a'));
        assert_eq!(uart.read_byte(), Some(b'b'));
        assert_eq!(uart.read_byte(), None);
    }
}
