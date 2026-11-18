//! Local APIC: deslocamentos, valores de registrador e a sequência de uso, sobre
//! uma porta de registradores abstrata (MSR no x2APIC, MMIO no xAPIC). Puro.

/// Vetores da IDT: ISA nas IRQs, depois das exceções.
pub const VECTOR_TIMER: u8 = 0x20;
pub const VECTOR_KEYBOARD: u8 = 0x21;
pub const VECTOR_SERIAL: u8 = 0x24;
pub const VECTOR_SPURIOUS: u8 = 0xFF;
/// IPIs: refazer a decisão de escalonamento e parar a CPU (pânico em outro núcleo).
pub const VECTOR_RESCHEDULE: u8 = 0xFD;
pub const VECTOR_STOP: u8 = 0xFE;

pub const ID: usize = 0x20;
pub const EOI: usize = 0xB0;
pub const SPURIOUS: usize = 0xF0;
pub const ICR_LOW: usize = 0x300;
pub const ICR_HIGH: usize = 0x310;
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
/// Modo TSC-deadline: bits 18:17 do LVT em `10b` (`APIC_LVT_TIMER_TSCDEADLINE` do Linux).
const LVT_TSC_DEADLINE: u32 = 2 << 17;
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

/// ICR (SDM vol. 3A, seção 11.6.1): entrega fixa, destino físico, nível "assert", borda.
const ICR_ASSERT: u32 = 1 << 14;
/// Bit de "envio pendente" do ICR no xAPIC; no x2APIC ele deixou de existir.
const ICR_DELIVERY_PENDING: u32 = 1 << 12;
/// Atalho de destino "todos menos eu" (bits 19:18 do ICR).
const ICR_ALL_BUT_SELF: u32 = 0b11 << 18;
/// Voltas de espera pelo envio anterior no xAPIC antes de desistir.
const ICR_POLL_LIMIT: u32 = 1_000_000;

/// Acesso aos registradores do Local APIC pelo deslocamento do modo xAPIC.
pub trait ApicRegisters {
    fn read(&self, offset: usize) -> u32;
    fn write(&self, offset: usize, value: u32);

    /// Escreve o ICR: no xAPIC são dois registradores de 32 bits, o alto (destino em
    /// 31:24) primeiro; o x2APIC troca por uma escrita de 64 bits.
    fn write_icr(&self, destination: u32, low: u32) {
        self.write(ICR_HIGH, destination << 24);
        self.write(ICR_LOW, low);
    }

    /// O ICR ainda entrega o IPI anterior (só o xAPIC tem esse bit).
    fn icr_pending(&self) -> bool {
        self.read(ICR_LOW) & ICR_DELIVERY_PENDING != 0
    }
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

    /// Timer de um tiro: divisor 16 e vetor `VECTOR_TIMER`, desarmado até `arm_oneshot`.
    pub fn set_oneshot(&self) {
        self.regs.write(TIMER_DIVIDE, DIVIDE_BY_16);
        self.regs.write(LVT_TIMER, u32::from(VECTOR_TIMER));
    }

    /// Dispara uma vez, `count` ticks do contador (de 1 em diante) depois de agora.
    pub fn arm_oneshot(&self, count: u32) {
        self.regs.write(TIMER_INITIAL, count.max(1));
    }

    /// Timer em modo TSC-deadline, que se arma pelo MSR `IA32_TSC_DEADLINE`.
    pub fn set_tsc_deadline(&self) {
        self.regs
            .write(LVT_TIMER, LVT_TSC_DEADLINE | u32::from(VECTOR_TIMER));
    }

    fn send_icr(&self, destination: u32, low: u32) {
        for _ in 0..ICR_POLL_LIMIT {
            if !self.regs.icr_pending() {
                break;
            }
            core::hint::spin_loop();
        }
        self.regs.write_icr(destination, low);
    }

    /// IPI de entrega fixa, vetor `vector`, à CPU de LAPIC ID `destination`.
    pub fn send_ipi(&self, destination: u32, vector: u8) {
        self.send_icr(destination, ICR_ASSERT | u32::from(vector));
    }

    /// O mesmo IPI a todas as CPUs menos esta (atalho do ICR, sem destino).
    pub fn send_ipi_all_but_self(&self, vector: u8) {
        self.send_icr(0, ICR_ASSERT | ICR_ALL_BUT_SELF | u32::from(vector));
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
                ICR_LOW => 0,
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
    fn oneshot_selects_mode_zero_and_arms_with_the_count() {
        let regs = FakeRegs::default();
        let apic = LocalApic::new(&regs);
        apic.set_oneshot();
        apic.arm_oneshot(1000);
        apic.arm_oneshot(0);
        assert_eq!(
            *regs.writes.borrow(),
            [
                (TIMER_DIVIDE, 0x3),
                (LVT_TIMER, 0x20),
                (TIMER_INITIAL, 1000),
                (TIMER_INITIAL, 1),
            ]
        );
    }

    #[test]
    fn tsc_deadline_mode_is_bits_18_17_equal_to_two() {
        let regs = FakeRegs::default();
        LocalApic::new(&regs).set_tsc_deadline();
        assert_eq!(*regs.writes.borrow(), [(LVT_TIMER, 0x4_0020)]);
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
    fn ipi_writes_destination_then_vector_in_the_xapic_layout() {
        let regs = FakeRegs::default();
        LocalApic::new(&regs).send_ipi(3, VECTOR_RESCHEDULE);
        assert_eq!(
            *regs.writes.borrow(),
            [(ICR_HIGH, 3 << 24), (ICR_LOW, 0x4000 | 0xFD)]
        );
    }

    #[test]
    fn broadcast_ipi_uses_the_all_but_self_shorthand() {
        let regs = FakeRegs::default();
        LocalApic::new(&regs).send_ipi_all_but_self(VECTOR_STOP);
        assert_eq!(regs.writes.borrow()[1], (ICR_LOW, 0xC_0000 | 0x4000 | 0xFE));
    }

    #[test]
    fn x2apic_icr_register_is_msr_830() {
        assert_eq!(x2apic_msr(ICR_LOW), 0x830);
    }

    #[test]
    fn ipi_vectors_do_not_collide_with_the_others() {
        let all = [
            VECTOR_TIMER,
            VECTOR_KEYBOARD,
            VECTOR_SERIAL,
            VECTOR_RESCHEDULE,
            VECTOR_STOP,
            VECTOR_SPURIOUS,
        ];
        for (i, a) in all.iter().enumerate() {
            assert!(all[i + 1..].iter().all(|b| a != b));
        }
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
