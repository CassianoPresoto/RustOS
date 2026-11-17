//! Números de interrupção do GIC que o adaptador conhece.

/// PPI do temporizador virtual (`CNTV`): INTID 27, o da BSA e da `virt` da QEMU.
pub const VIRTUAL_TIMER_INTID: u32 = 27;

/// UART0 da `virt` é a SPI 1, INTID 33 (DEBT-0002).
pub const QEMU_VIRT_PL011_INTID: u32 = 33;

/// Do 1020 em diante o `ICC_IAR1_EL1` devolve INTIDs especiais (1023: espúrio).
const SPECIAL_INTID_FIRST: u32 = 1020;

/// O INTID dentro do valor lido de `ICC_IAR1_EL1` (bits 23:0).
pub const fn intid_from_iar(iar: u64) -> u32 {
    (iar & 0x00ff_ffff) as u32
}

/// INTID especial não é interrupção de verdade e não leva EOI.
pub const fn needs_end_of_interrupt(intid: u32) -> bool {
    intid < SPECIAL_INTID_FIRST
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iar_keeps_only_the_intid_bits() {
        assert_eq!(intid_from_iar(0xff00_001b), 27);
        assert_eq!(intid_from_iar(0x3ff), 1023);
    }

    #[test]
    fn special_intids_get_no_eoi() {
        assert!(needs_end_of_interrupt(27));
        assert!(needs_end_of_interrupt(1019));
        assert!(!needs_end_of_interrupt(1020));
        assert!(!needs_end_of_interrupt(1023));
    }
}
