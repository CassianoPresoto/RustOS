//! Controlador PS/2 i8042 do PC: configura a porta 1 e entrega as teclas como
//! `KeyEvent`. Consulta por espera limitada; a IRQ 1 só liga no controlador.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

mod scancode;

use core::hint::spin_loop;

use rustos_hal::{KeyEvent, RegisterBus};

pub use scancode::Set1Decoder;

const DATA: usize = 0;
const STATUS_COMMAND: usize = 4;

const STATUS_OUTPUT_FULL: u8 = 1 << 0;
const STATUS_INPUT_FULL: u8 = 1 << 1;
const STATUS_AUX_DATA: u8 = 1 << 5;
const STATUS_TIMEOUT: u8 = 1 << 6;
const STATUS_PARITY: u8 = 1 << 7;

const CMD_READ_CONFIG: u8 = 0x20;
const CMD_WRITE_CONFIG: u8 = 0x60;
const CMD_DISABLE_AUX: u8 = 0xA7;
const CMD_SELF_TEST: u8 = 0xAA;
const CMD_DISABLE_KBD: u8 = 0xAD;
const CMD_ENABLE_KBD: u8 = 0xAE;

const CONFIG_KBD_IRQ: u8 = 1 << 0;
const CONFIG_AUX_IRQ: u8 = 1 << 1;
const CONFIG_KBD_DISABLED: u8 = 1 << 4;
const CONFIG_AUX_DISABLED: u8 = 1 << 5;
const CONFIG_TRANSLATE: u8 = 1 << 6;

const SELF_TEST_OK: u8 = 0x55;
const SELF_TEST_TRIES: u8 = 5;
const FLUSH_LIMIT: u8 = 16;
const WAIT_LIMIT: u32 = 100_000;
const BYTES_PER_READ: u8 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum I8042Error {
    /// O controlador não respondeu dentro do limite de tentativas.
    Timeout,
    /// O buffer de saída não esvaziou: não há controlador ou ele travou.
    StuckOutput,
    /// O autoteste respondeu outro valor que não 0x55.
    SelfTestFailed(u8),
}

#[derive(Debug)]
pub struct I8042<B> {
    bus: B,
    decoder: Set1Decoder,
}

impl<B: RegisterBus> I8042<B> {
    pub const fn new(bus: B) -> Self {
        Self {
            bus,
            decoder: Set1Decoder::new(),
        }
    }

    /// Configura o controlador: porta 1 ligada, tradução para o conjunto 1, IRQ 1 ligada.
    /// A porta 2 (mouse) fica desligada e sem IRQ; o teclado em si não recebe comando (DEBT-0009).
    pub fn init(&mut self) -> Result<(), I8042Error> {
        self.decoder.reset();
        self.command(CMD_DISABLE_KBD)?;
        self.command(CMD_DISABLE_AUX)?;
        self.flush()?;
        self.self_test()?;

        self.command(CMD_READ_CONFIG)?;
        let config = self.read_data()?;
        let quiet = (config | CONFIG_KBD_DISABLED | CONFIG_AUX_DISABLED | CONFIG_TRANSLATE)
            & !(CONFIG_KBD_IRQ | CONFIG_AUX_IRQ);
        self.write_config(quiet)?;
        self.flush()?;

        self.command(CMD_ENABLE_KBD)?;
        self.write_config((quiet | CONFIG_KBD_IRQ) & !CONFIG_KBD_DISABLED)
    }

    /// Próximo evento já decodificado; `None` se o buffer de saída está vazio.
    pub fn read_event(&mut self) -> Option<KeyEvent> {
        for _ in 0..BYTES_PER_READ {
            let status = self.bus.read8(STATUS_COMMAND);
            if status & STATUS_OUTPUT_FULL == 0 {
                return None;
            }
            let byte = self.bus.read8(DATA);
            if status & (STATUS_PARITY | STATUS_TIMEOUT) != 0 {
                self.decoder.reset();
            } else if status & STATUS_AUX_DATA == 0
                && let Some(event) = self.decoder.feed(byte)
            {
                return Some(event);
            }
        }
        None
    }

    fn wait_input_clear(&self) -> Result<(), I8042Error> {
        for _ in 0..WAIT_LIMIT {
            if self.bus.read8(STATUS_COMMAND) & STATUS_INPUT_FULL == 0 {
                return Ok(());
            }
            spin_loop();
        }
        Err(I8042Error::Timeout)
    }

    fn wait_output_full(&self) -> Result<(), I8042Error> {
        for _ in 0..WAIT_LIMIT {
            if self.bus.read8(STATUS_COMMAND) & STATUS_OUTPUT_FULL != 0 {
                return Ok(());
            }
            spin_loop();
        }
        Err(I8042Error::Timeout)
    }

    fn command(&self, command: u8) -> Result<(), I8042Error> {
        self.wait_input_clear()?;
        self.bus.write8(STATUS_COMMAND, command);
        Ok(())
    }

    fn write_data(&self, value: u8) -> Result<(), I8042Error> {
        self.wait_input_clear()?;
        self.bus.write8(DATA, value);
        Ok(())
    }

    fn read_data(&self) -> Result<u8, I8042Error> {
        self.wait_output_full()?;
        Ok(self.bus.read8(DATA))
    }

    fn write_config(&self, config: u8) -> Result<(), I8042Error> {
        self.command(CMD_WRITE_CONFIG)?;
        self.write_data(config)
    }

    fn flush(&self) -> Result<(), I8042Error> {
        for _ in 0..FLUSH_LIMIT {
            if self.bus.read8(STATUS_COMMAND) & STATUS_OUTPUT_FULL == 0 {
                return Ok(());
            }
            self.bus.read8(DATA);
        }
        Err(I8042Error::StuckOutput)
    }

    fn self_test(&self) -> Result<(), I8042Error> {
        let mut answer = 0;
        for _ in 0..SELF_TEST_TRIES {
            self.command(CMD_SELF_TEST)?;
            answer = self.read_data()?;
            if answer == SELF_TEST_OK {
                return Ok(());
            }
        }
        Err(I8042Error::SelfTestFailed(answer))
    }
}

#[cfg(test)]
mod tests {
    use core::cell::RefCell;
    use std::collections::VecDeque;

    use rustos_hal::KeyCode;

    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Op {
        Command(u8),
        Data(u8),
        ReadData,
    }

    struct Inner {
        ops: Vec<Op>,
        output: VecDeque<(u8, u8)>,
        config: u8,
        self_test: VecDeque<u8>,
        input_full: bool,
        refill: bool,
        mute: bool,
    }

    struct FakeBus(RefCell<Inner>);

    impl FakeBus {
        fn new() -> Self {
            Self(RefCell::new(Inner {
                ops: Vec::new(),
                output: VecDeque::new(),
                config: 0x45,
                self_test: VecDeque::from([SELF_TEST_OK]),
                input_full: false,
                refill: false,
                mute: false,
            }))
        }

        fn with_bytes(bytes: &[u8]) -> Self {
            let bus = Self::new();
            bus.0.borrow_mut().output = bytes.iter().map(|&b| (0, b)).collect();
            bus
        }

        fn ops(&self) -> Vec<Op> {
            self.0.borrow().ops.clone()
        }
    }

    impl RegisterBus for FakeBus {
        fn read8(&self, offset: usize) -> u8 {
            let mut inner = self.0.borrow_mut();
            if offset == DATA {
                inner.ops.push(Op::ReadData);
                let byte = inner.output.pop_front().map_or(0, |(_, b)| b);
                if inner.refill {
                    inner.output.push_back((0, 0x1E));
                }
                return byte;
            }
            assert_eq!(offset, STATUS_COMMAND);
            let mut status = 0;
            if let Some(&(flags, _)) = inner.output.front() {
                status |= STATUS_OUTPUT_FULL | flags;
            }
            if inner.input_full {
                status |= STATUS_INPUT_FULL;
            }
            status
        }

        fn write8(&self, offset: usize, value: u8) {
            let mut inner = self.0.borrow_mut();
            if offset == DATA {
                inner.ops.push(Op::Data(value));
                return;
            }
            assert_eq!(offset, STATUS_COMMAND);
            inner.ops.push(Op::Command(value));
            if inner.mute {
                return;
            }
            match value {
                CMD_READ_CONFIG => {
                    let config = inner.config;
                    inner.output.push_back((0, config));
                }
                CMD_SELF_TEST => {
                    let answer = inner.self_test.pop_front().unwrap_or(0xFC);
                    inner.output.push_back((0, answer));
                }
                _ => {}
            }
        }

        fn read32(&self, _: usize) -> u32 {
            unreachable!("o i8042 só usa portas de 8 bits")
        }

        fn write32(&self, _: usize, _: u32) {
            unreachable!("o i8042 só usa portas de 8 bits")
        }
    }

    fn keyboard(bytes: &[u8]) -> I8042<FakeBus> {
        I8042::new(FakeBus::with_bytes(bytes))
    }

    fn events(bytes: &[u8]) -> Vec<(u8, bool)> {
        let mut kbd = keyboard(bytes);
        let mut out = Vec::new();
        while let Some(e) = kbd.read_event() {
            out.push((e.code.0, e.pressed));
        }
        out
    }

    fn make_break(codes: &[(u8, u8)]) {
        for &(scancode, hid) in codes {
            assert_eq!(events(&[scancode]), [(hid, true)], "make {scancode:#04x}");
            assert_eq!(
                events(&[scancode | 0x80]),
                [(hid, false)],
                "break {scancode:#04x}"
            );
        }
    }

    fn make_break_extended(codes: &[(u8, u8)]) {
        for &(scancode, hid) in codes {
            assert_eq!(
                events(&[0xE0, scancode]),
                [(hid, true)],
                "E0 {scancode:#04x}"
            );
            assert_eq!(
                events(&[0xE0, scancode | 0x80]),
                [(hid, false)],
                "E0 break {scancode:#04x}"
            );
        }
    }

    #[test]
    fn init_follows_the_controller_sequence() {
        let mut kbd = keyboard(&[]);
        assert_eq!(kbd.init(), Ok(()));
        assert_eq!(
            kbd.bus.ops(),
            [
                Op::Command(CMD_DISABLE_KBD),
                Op::Command(CMD_DISABLE_AUX),
                Op::Command(CMD_SELF_TEST),
                Op::ReadData,
                Op::Command(CMD_READ_CONFIG),
                Op::ReadData,
                Op::Command(CMD_WRITE_CONFIG),
                Op::Data(0x74),
                Op::Command(CMD_ENABLE_KBD),
                Op::Command(CMD_WRITE_CONFIG),
                Op::Data(0x65),
            ]
        );
    }

    #[test]
    fn init_turns_translation_and_irq_on_and_keeps_the_mouse_off() {
        let bus = FakeBus::new();
        bus.0.borrow_mut().config = 0x02 | 0x04;
        let mut kbd = I8042::new(bus);
        assert_eq!(kbd.init(), Ok(()));
        let ops = kbd.bus.ops();
        let Some(&Op::Data(last)) = ops.last() else {
            panic!("a última escrita deve ser o byte de configuração");
        };
        assert_eq!(last & CONFIG_TRANSLATE, CONFIG_TRANSLATE);
        assert_eq!(last & CONFIG_KBD_IRQ, CONFIG_KBD_IRQ);
        assert_eq!(last & CONFIG_KBD_DISABLED, 0);
        assert_eq!(last & CONFIG_AUX_IRQ, 0);
        assert_eq!(last & CONFIG_AUX_DISABLED, CONFIG_AUX_DISABLED);
        assert_eq!(last & 0x04, 0x04, "o bit de sistema do BIOS é preservado");
    }

    #[test]
    fn init_drains_stale_output_before_the_self_test() {
        let mut kbd = keyboard(&[0x1E, 0x9E, 0xFA]);
        assert_eq!(kbd.init(), Ok(()));
        let ops = kbd.bus.ops();
        let first_test = ops.iter().position(|&o| o == Op::Command(CMD_SELF_TEST));
        let reads_before = ops[..first_test.unwrap()]
            .iter()
            .filter(|&&o| o == Op::ReadData)
            .count();
        assert_eq!(reads_before, 3);
    }

    #[test]
    fn init_gives_up_when_the_output_buffer_never_drains() {
        let bus = FakeBus::with_bytes(&[0x1E]);
        bus.0.borrow_mut().refill = true;
        let mut kbd = I8042::new(bus);
        assert_eq!(kbd.init(), Err(I8042Error::StuckOutput));
    }

    #[test]
    fn init_retries_the_self_test_before_failing() {
        let bus = FakeBus::new();
        bus.0.borrow_mut().self_test = VecDeque::from([0x00, 0x00, SELF_TEST_OK]);
        let mut kbd = I8042::new(bus);
        assert_eq!(kbd.init(), Ok(()));
        let tests = kbd
            .bus
            .ops()
            .iter()
            .filter(|&&o| o == Op::Command(CMD_SELF_TEST))
            .count();
        assert_eq!(tests, 3);
    }

    #[test]
    fn init_reports_the_wrong_self_test_answer() {
        let bus = FakeBus::new();
        bus.0.borrow_mut().self_test = VecDeque::from([0xFC; 5]);
        let mut kbd = I8042::new(bus);
        assert_eq!(kbd.init(), Err(I8042Error::SelfTestFailed(0xFC)));
        let ops = kbd.bus.ops();
        assert!(!ops.contains(&Op::Command(CMD_ENABLE_KBD)));
    }

    #[test]
    fn init_times_out_when_the_input_buffer_stays_full() {
        let bus = FakeBus::new();
        bus.0.borrow_mut().input_full = true;
        let mut kbd = I8042::new(bus);
        assert_eq!(kbd.init(), Err(I8042Error::Timeout));
        assert_eq!(kbd.bus.ops().len(), 0);
    }

    #[test]
    fn init_times_out_when_the_controller_does_not_answer() {
        let bus = FakeBus::new();
        bus.0.borrow_mut().mute = true;
        let mut kbd = I8042::new(bus);
        assert_eq!(kbd.init(), Err(I8042Error::Timeout));
    }

    #[test]
    fn read_event_is_none_when_the_buffer_is_empty() {
        let mut kbd = keyboard(&[]);
        assert_eq!(kbd.read_event(), None);
        assert_eq!(kbd.bus.ops().len(), 0);
    }

    #[test]
    fn letters_and_digits_decode_to_hid_usages() {
        make_break(&[
            (0x1E, 0x04),
            (0x30, 0x05),
            (0x2E, 0x06),
            (0x20, 0x07),
            (0x12, 0x08),
            (0x21, 0x09),
            (0x22, 0x0A),
            (0x23, 0x0B),
            (0x17, 0x0C),
            (0x24, 0x0D),
            (0x25, 0x0E),
            (0x26, 0x0F),
            (0x32, 0x10),
            (0x31, 0x11),
            (0x18, 0x12),
            (0x19, 0x13),
            (0x10, 0x14),
            (0x13, 0x15),
            (0x1F, 0x16),
            (0x14, 0x17),
            (0x16, 0x18),
            (0x2F, 0x19),
            (0x11, 0x1A),
            (0x2D, 0x1B),
            (0x15, 0x1C),
            (0x2C, 0x1D),
            (0x02, 0x1E),
            (0x0A, 0x26),
            (0x0B, 0x27),
        ]);
    }

    #[test]
    fn punctuation_and_editing_keys_decode_to_hid_usages() {
        make_break(&[
            (0x01, 0x29),
            (0x0C, 0x2D),
            (0x0D, 0x2E),
            (0x0E, 0x2A),
            (0x0F, 0x2B),
            (0x1A, 0x2F),
            (0x1B, 0x30),
            (0x1C, 0x28),
            (0x27, 0x33),
            (0x28, 0x34),
            (0x29, 0x35),
            (0x2B, 0x31),
            (0x33, 0x36),
            (0x34, 0x37),
            (0x35, 0x38),
            (0x39, 0x2C),
            (0x3A, 0x39),
        ]);
    }

    #[test]
    fn abnt2_extra_keys_decode_to_the_international_usages() {
        make_break(&[(0x56, 0x64), (0x73, 0x87), (0x7E, 0x85)]);
    }

    #[test]
    fn function_keys_decode_in_order_from_f1_to_f12() {
        make_break(&[
            (0x3B, 0x3A),
            (0x3C, 0x3B),
            (0x3D, 0x3C),
            (0x3E, 0x3D),
            (0x3F, 0x3E),
            (0x40, 0x3F),
            (0x41, 0x40),
            (0x42, 0x41),
            (0x43, 0x42),
            (0x44, 0x43),
            (0x57, 0x44),
            (0x58, 0x45),
        ]);
    }

    #[test]
    fn modifiers_decode_with_the_left_and_right_variants() {
        make_break(&[(0x1D, 0xE0), (0x2A, 0xE1), (0x38, 0xE2), (0x36, 0xE5)]);
        make_break_extended(&[(0x1D, 0xE4), (0x38, 0xE6), (0x5B, 0xE3), (0x5C, 0xE7)]);
    }

    #[test]
    fn keypad_keys_decode_with_the_extended_slash_and_enter() {
        make_break(&[
            (0x45, 0x53),
            (0x37, 0x55),
            (0x4A, 0x56),
            (0x4E, 0x57),
            (0x47, 0x5F),
            (0x48, 0x60),
            (0x49, 0x61),
            (0x4B, 0x5C),
            (0x4C, 0x5D),
            (0x4D, 0x5E),
            (0x4F, 0x59),
            (0x50, 0x5A),
            (0x51, 0x5B),
            (0x52, 0x62),
            (0x53, 0x63),
        ]);
        make_break_extended(&[(0x35, 0x54), (0x1C, 0x58)]);
    }

    #[test]
    fn navigation_keys_need_the_e0_prefix() {
        make_break_extended(&[
            (0x48, 0x52),
            (0x50, 0x51),
            (0x4B, 0x50),
            (0x4D, 0x4F),
            (0x47, 0x4A),
            (0x4F, 0x4D),
            (0x49, 0x4B),
            (0x51, 0x4E),
            (0x52, 0x49),
            (0x53, 0x4C),
            (0x5D, 0x65),
        ]);
        assert_eq!(
            events(&[0x48]),
            [(0x60, true)],
            "sem E0 é o 8 do teclado numérico"
        );
    }

    #[test]
    fn the_fake_shift_of_print_screen_generates_no_event() {
        assert_eq!(
            events(&[0xE0, 0x2A, 0xE0, 0x37, 0xE0, 0xB7, 0xE0, 0xAA]),
            [(0x46, true), (0x46, false)]
        );
        assert_eq!(
            events(&[0xE0, 0x2A, 0xE0, 0x48, 0xE0, 0xC8, 0xE0, 0xAA]),
            [(0x52, true), (0x52, false)]
        );
    }

    #[test]
    fn pause_is_consumed_without_leaving_garbage() {
        let bytes = [0xE1, 0x1D, 0x45, 0xE1, 0x9D, 0xC5, 0x1E, 0x9E];
        assert_eq!(events(&bytes), [(0x04, true), (0x04, false)]);
    }

    #[test]
    fn a_prefix_split_across_reads_is_remembered() {
        let mut kbd = keyboard(&[0xE0]);
        assert_eq!(kbd.read_event(), None);
        kbd.bus.0.borrow_mut().output.push_back((0, 0x4B));
        assert_eq!(
            kbd.read_event(),
            Some(KeyEvent {
                code: KeyCode::LEFT,
                pressed: true
            })
        );
    }

    #[test]
    fn bytes_that_are_not_keys_are_ignored() {
        assert_eq!(events(&[0xFA, 0x00, 0xFF, 0x7F, 0x1E]), [(0x04, true)]);
        assert_eq!(events(&[0xE0, 0x00, 0x1E]), [(0x04, true)]);
    }

    #[test]
    fn mouse_bytes_and_line_errors_are_dropped() {
        let bus = FakeBus::new();
        {
            let mut inner = bus.0.borrow_mut();
            inner.output.push_back((STATUS_AUX_DATA, 0x1E));
            inner.output.push_back((0, 0xE0));
            inner.output.push_back((STATUS_PARITY, 0x4B));
            inner.output.push_back((0, 0x1E));
        }
        let mut kbd = I8042::new(bus);
        assert_eq!(
            kbd.read_event(),
            Some(KeyEvent {
                code: KeyCode::A,
                pressed: true
            })
        );
        assert_eq!(kbd.read_event(), None);
    }

    #[test]
    fn a_stuck_data_port_does_not_loop_forever() {
        let bus = FakeBus::with_bytes(&[0xFA]);
        bus.0.borrow_mut().refill = false;
        let mut kbd = I8042::new(bus);
        for _ in 0..1000 {
            kbd.bus.0.borrow_mut().output.push_back((0, 0xFA));
        }
        assert_eq!(kbd.read_event(), None);
        let reads = kbd.bus.ops().len();
        assert_eq!(reads, usize::from(BYTES_PER_READ));
    }

    #[test]
    fn no_two_scancodes_decode_to_the_same_usage() {
        let mut seen = [false; 256];
        for make in 0..0x80u8 {
            if let Some(e) = Set1Decoder::new().feed(make) {
                assert!(
                    !seen[usize::from(e.code.0)],
                    "HID {:#04x} repetido",
                    e.code.0
                );
                seen[usize::from(e.code.0)] = true;
            }
        }
        assert!(seen.iter().filter(|&&s| s).count() >= 80);
    }
}
