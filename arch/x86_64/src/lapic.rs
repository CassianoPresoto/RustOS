//! Local APIC: deslocamentos, valores de registrador e a sequência de uso, sobre
//! uma porta de registradores abstrata (MSR no x2APIC, MMIO no xAPIC). Puro.

/// Vetores da IDT: ISA nas IRQs, depois das exceções.
pub const VECTOR_TIMER: u8 = 0x20;
pub const VECTOR_KEYBOARD: u8 = 0x21;
pub const VECTOR_SERIAL: u8 = 0x24;
pub const VECTOR_SPURIOUS: u8 = 0xFF;

pub const ID: usize = 0x20;
pub const EOI: usize = 0xB0;
pub const SPURIOUS: usize = 0xF0;
pub const LVT_TIMER: usize = 0x320;
pub const TIMER_INITIAL: usize = 0x380;
pub const TIMER_CURRENT: usize = 0x390;
pub const TIMER_DIVIDE: usize = 0x3E0;

/// `IA32_APIC_BASE` e seus bits (SDM vol. 3A, seção 11.4.4).
pub const MSR_APIC_BASE: u32 = 0x1B;
pub const APIC_BASE_ENABLE: u64 = 1 << 11;
pub const APIC_BASE_X2APIC: u64 = 1 << 10;
pub const APIC_BASE_ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;

const MSR_X2APIC_BASE: u32 = 0x800;
const SPURIOUS_ENABLE: u32 = 1 << 8;
const LVT_MASKED: u32 = 1 << 16;
const LVT_PERIODIC: u32 = 1 << 17;
/// Divisor 16 no `TIMER_DIVIDE`: bits 3, 1 e 0 valem 0b0011 (SDM vol. 3A, figura 11-8).
const DIVIDE_BY_16: u32 = 0x3;
pub const TIMER_DIVISOR: u64 = 16;

/// Endereço do registrador no modo x2APIC: `0x800 + (deslocamento MMIO >> 4)`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "o deslocamento é menor que 0x1000"
)]
pub const fn x2apic_msr(offset: usize) -> u32 {
    MSR_X2APIC_BASE + (offset >> 4) as u32
}

/// Acesso aos registradores do Local APIC pelo deslocamento do modo xAPIC.
pub trait ApicRegisters {
    fn read(&self, offset: usize) -> u32;
    fn write(&self, offset: usize, value: u32);
}

#[derive(Clone, Copy, Debug)]
pub struct LocalApic<R> {
    regs: R,
}

impl<R: ApicRegisters> LocalApic<R> {
    pub const fn new(regs: R) -> Self {
        Self { regs }
    }

    /// Liga o APIC por software com o vetor espúrio e deixa o timer mascarado.
    pub fn enable(&self) {
        self.regs
            .write(SPURIOUS, SPURIOUS_ENABLE | u32::from(VECTOR_SPURIOUS));
        self.regs.write(LVT_TIMER, LVT_MASKED);
    }

    /// No x2APIC o registrador de ID tem 32 bits; no xAPIC o ID fica nos 8 de cima.
    pub fn id(&self, x2apic: bool) -> u32 {
        let raw = self.regs.read(ID);
        if x2apic { raw } else { raw >> 24 }
    }

    pub fn eoi(&self) {
        self.regs.write(EOI, 0);
    }

    /// Conta de `u32::MAX` para baixo, sem interrupção, para medir um intervalo.
    pub fn start_countdown(&self) {
        self.regs.write(TIMER_DIVIDE, DIVIDE_BY_16);
        self.regs
            .write(LVT_TIMER, LVT_MASKED | u32::from(VECTOR_TIMER));
        self.regs.write(TIMER_INITIAL, u32::MAX);
    }

    /// Ticks do contador desde `start_countdown`.
    pub fn elapsed_ticks(&self) -> u32 {
        u32::MAX - self.regs.read(TIMER_CURRENT)
    }

    /// Timer periódico que dispara `VECTOR_TIMER` a cada `initial` ticks do contador.
    pub fn start_periodic(&self, initial: u32) {
        self.regs.write(TIMER_DIVIDE, DIVIDE_BY_16);
        self.regs
            .write(LVT_TIMER, LVT_PERIODIC | u32::from(VECTOR_TIMER));
        self.regs.write(TIMER_INITIAL, initial);
    }

    pub fn stop_timer(&self) {
        self.regs.write(TIMER_INITIAL, 0);
        self.regs.write(LVT_TIMER, LVT_MASKED);
    }
}

#[cfg(test)]
mod tests {
    use core::cell::{Cell, RefCell};

    use super::*;

    #[derive(Default)]
    struct FakeRegs {
        writes: RefCell<Vec<(usize, u32)>>,
        current: Cell<u32>,
        id: Cell<u32>,
    }

    impl ApicRegisters for &FakeRegs {
        fn read(&self, offset: usize) -> u32 {
            match offset {
                TIMER_CURRENT => self.current.get(),
                ID => self.id.get(),
                _ => panic!("leitura inesperada em {offset:#x}"),
            }
        }

        fn write(&self, offset: usize, value: u32) {
            self.writes.borrow_mut().push((offset, value));
        }
    }

    #[test]
    fn x2apic_msrs_follow_the_manual() {
        assert_eq!(x2apic_msr(ID), 0x802);
        assert_eq!(x2apic_msr(EOI), 0x80B);
        assert_eq!(x2apic_msr(SPURIOUS), 0x80F);
        assert_eq!(x2apic_msr(LVT_TIMER), 0x832);
        assert_eq!(x2apic_msr(TIMER_INITIAL), 0x838);
        assert_eq!(x2apic_msr(TIMER_CURRENT), 0x839);
        assert_eq!(x2apic_msr(TIMER_DIVIDE), 0x83E);
    }

    #[test]
    fn enable_sets_the_software_enable_bit_and_spurious_vector() {
        let regs = FakeRegs::default();
        LocalApic::new(&regs).enable();
        assert_eq!(regs.writes.borrow()[0], (SPURIOUS, 0x1FF));
        assert_eq!(regs.writes.borrow()[1], (LVT_TIMER, LVT_MASKED));
    }

    #[test]
    fn periodic_timer_programs_divide_mode_and_count_in_order() {
        let regs = FakeRegs::default();
        LocalApic::new(&regs).start_periodic(625_000);
        assert_eq!(
            *regs.writes.borrow(),
            [
                (TIMER_DIVIDE, 0x3),
                (LVT_TIMER, LVT_PERIODIC | 0x20),
                (TIMER_INITIAL, 625_000),
            ]
        );
    }

    #[test]
    fn countdown_is_masked_and_elapsed_counts_down_from_max() {
        let regs = FakeRegs::default();
        let apic = LocalApic::new(&regs);
        apic.start_countdown();
        assert_eq!(regs.writes.borrow()[1], (LVT_TIMER, LVT_MASKED | 0x20));
        assert_eq!(regs.writes.borrow()[2], (TIMER_INITIAL, u32::MAX));
        regs.current.set(u32::MAX - 1234);
        assert_eq!(apic.elapsed_ticks(), 1234);
    }

    #[test]
    fn eoi_writes_zero() {
        let regs = FakeRegs::default();
        LocalApic::new(&regs).eoi();
        assert_eq!(*regs.writes.borrow(), [(EOI, 0)]);
    }

    #[test]
    fn id_position_depends_on_the_mode() {
        let regs = FakeRegs::default();
        regs.id.set(0x0300_0000);
        let apic = LocalApic::new(&regs);
        assert_eq!(apic.id(false), 3);
        regs.id.set(3);
        assert_eq!(apic.id(true), 3);
    }

    #[test]
    fn stop_timer_zeroes_the_count_and_masks() {
        let regs = FakeRegs::default();
        LocalApic::new(&regs).stop_timer();
        assert_eq!(
            *regs.writes.borrow(),
            [(TIMER_INITIAL, 0), (LVT_TIMER, LVT_MASKED)]
        );
    }
}
