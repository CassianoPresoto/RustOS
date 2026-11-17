//! UART 16550: COM1 do PC por porta de E/S, ou MMIO em placas RISC-V e ARM.
//! Transmissão por espera ativa e recepção por consulta; interrupção entra na fase 1.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::hint::spin_loop;

use rustos_hal::{ByteSource, Console, RegisterBus};

const DATA: usize = 0;
const INTERRUPT_ENABLE: usize = 1;
const DIVISOR_LOW: usize = 0;
const DIVISOR_HIGH: usize = 1;
const FIFO_CONTROL: usize = 2;
const LINE_CONTROL: usize = 3;
const MODEM_CONTROL: usize = 4;
const LINE_STATUS: usize = 5;

const LINE_DLAB: u8 = 1 << 7;
const LINE_8N1: u8 = 0b11;
const FIFO_ENABLE_AND_CLEAR: u8 = 0xC7;
const MODEM_DTR_RTS: u8 = 0b11;
const STATUS_DATA_READY: u8 = 1 << 0;
const STATUS_TX_EMPTY: u8 = 1 << 5;

/// Divisor de 115200 baud com o relógio padrão de 1,8432 MHz.
pub const DIVISOR_115200: u16 = 1;

#[derive(Debug)]
pub struct Uart16550<B> {
    bus: B,
}

impl<B: RegisterBus> Uart16550<B> {
    pub const fn new(bus: B) -> Self {
        Self { bus }
    }

    /// 8N1, FIFO ligada, sem interrupções.
    pub fn init(&mut self, divisor: u16) {
        let [low, high] = divisor.to_le_bytes();
        self.bus.write8(INTERRUPT_ENABLE, 0);
        self.bus.write8(LINE_CONTROL, LINE_DLAB);
        self.bus.write8(DIVISOR_LOW, low);
        self.bus.write8(DIVISOR_HIGH, high);
        self.bus.write8(LINE_CONTROL, LINE_8N1);
        self.bus.write8(FIFO_CONTROL, FIFO_ENABLE_AND_CLEAR);
        self.bus.write8(MODEM_CONTROL, MODEM_DTR_RTS);
    }

    pub fn write_byte(&mut self, byte: u8) {
        while self.bus.read8(LINE_STATUS) & STATUS_TX_EMPTY == 0 {
            spin_loop();
        }
        self.bus.write8(DATA, byte);
    }
}

impl<B: RegisterBus> Console for Uart16550<B> {
    fn write_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(byte);
        }
    }
}

impl<B: RegisterBus> ByteSource for Uart16550<B> {
    fn read_byte(&mut self) -> Option<u8> {
        (self.bus.read8(LINE_STATUS) & STATUS_DATA_READY != 0).then(|| self.bus.read8(DATA))
    }
}

#[cfg(test)]
mod tests {
    use core::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    use super::*;

    #[derive(Default)]
    struct FakeBus {
        writes: RefCell<Vec<(usize, u8)>>,
        busy_polls: Cell<u32>,
        received: RefCell<VecDeque<u8>>,
    }

    impl RegisterBus for FakeBus {
        fn read8(&self, offset: usize) -> u8 {
            if offset == DATA {
                return self
                    .received
                    .borrow_mut()
                    .pop_front()
                    .expect("leu DATA sem dado");
            }
            assert_eq!(offset, LINE_STATUS);
            let ready = if self.received.borrow().is_empty() {
                0
            } else {
                STATUS_DATA_READY
            };
            if self.busy_polls.get() > 0 {
                self.busy_polls.set(self.busy_polls.get() - 1);
                return ready;
            }
            STATUS_TX_EMPTY | ready
        }

        fn write8(&self, offset: usize, value: u8) {
            self.writes.borrow_mut().push((offset, value));
        }

        fn read32(&self, _: usize) -> u32 {
            unreachable!("16550 só usa registradores de 8 bits")
        }

        fn write32(&self, _: usize, _: u32) {
            unreachable!("16550 só usa registradores de 8 bits")
        }
    }

    fn data_bytes(bus: &FakeBus) -> Vec<u8> {
        bus.writes
            .borrow()
            .iter()
            .filter(|(off, _)| *off == DATA)
            .map(|(_, b)| *b)
            .collect()
    }

    #[test]
    fn init_programs_divisor_between_dlab_toggles() {
        let mut uart = Uart16550::new(FakeBus::default());
        uart.init(0x0102);
        let writes = uart.bus.writes.borrow();
        assert_eq!(writes[1], (LINE_CONTROL, LINE_DLAB));
        assert_eq!(writes[2], (DIVISOR_LOW, 0x02));
        assert_eq!(writes[3], (DIVISOR_HIGH, 0x01));
        assert_eq!(writes[4], (LINE_CONTROL, LINE_8N1));
    }

    #[test]
    fn newline_becomes_crlf() {
        let mut uart = Uart16550::new(FakeBus::default());
        uart.write_bytes(b"a\n");
        assert_eq!(data_bytes(&uart.bus), b"a\r\n");
    }

    #[test]
    fn waits_for_transmitter_before_writing() {
        let bus = FakeBus::default();
        bus.busy_polls.set(3);
        let mut uart = Uart16550::new(bus);
        uart.write_byte(b'x');
        assert_eq!(uart.bus.busy_polls.get(), 0);
        assert_eq!(data_bytes(&uart.bus), b"x");
    }

    #[test]
    fn read_byte_is_none_without_data_ready() {
        let mut uart = Uart16550::new(FakeBus::default());
        assert_eq!(uart.read_byte(), None);
    }

    #[test]
    fn read_byte_drains_the_receiver_in_order() {
        let bus = FakeBus::default();
        bus.received.borrow_mut().extend(*b"ok");
        let mut uart = Uart16550::new(bus);
        assert_eq!(uart.read_byte(), Some(b'o'));
        assert_eq!(uart.read_byte(), Some(b'k'));
        assert_eq!(uart.read_byte(), None);
    }
}
