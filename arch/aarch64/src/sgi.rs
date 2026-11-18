//! SGI (interrupção entre núcleos) do GICv3: INTIDs e o valor de `ICC_SGI1R_EL1`. Puro.

/// SGIs que o kernel usa: refazer o escalonamento e parar a CPU.
pub const SGI_RESCHEDULE: u32 = 0;
pub const SGI_STOP: u32 = 1;

/// Campos de `ICC_SGI1R_EL1` (Arm IHI 0069; `ICC_SGI1R_*` em `arm-gic-v3.h` do Linux).
const TARGET_LIST_MASK: u64 = 0xffff;
const AFF1_SHIFT: u32 = 16;
const INTID_SHIFT: u32 = 24;
const AFF2_SHIFT: u32 = 32;
const IRM: u64 = 1 << 40;
const RS_SHIFT: u32 = 44;
const AFF3_SHIFT: u32 = 48;
/// `Aff0` cabe numa lista de 16 bits por faixa (`RS`).
const LIST_BITS: u64 = 16;

/// Valor para mandar a SGI `intid` à CPU de afinidade `affinity` (`Aff3.Aff2.Aff1.Aff0`
/// em 32 bits, como `GICR_TYPER[63:32]`).
pub const fn sgi_to(intid: u32, affinity: u32) -> u64 {
    let aff0 = (affinity & 0xff) as u64;
    let aff1 = ((affinity >> 8) & 0xff) as u64;
    let aff2 = ((affinity >> 16) & 0xff) as u64;
    let aff3 = ((affinity >> 24) & 0xff) as u64;
    (1 << (aff0 % LIST_BITS)) & TARGET_LIST_MASK
        | aff1 << AFF1_SHIFT
        | (intid as u64 & 0xf) << INTID_SHIFT
        | aff2 << AFF2_SHIFT
        | (aff0 / LIST_BITS) << RS_SHIFT
        | aff3 << AFF3_SHIFT
}

/// Valor para mandar a SGI `intid` a todas as CPUs menos esta (`IRM = 1`).
pub const fn sgi_all_but_self(intid: u32) -> u64 {
    (intid as u64 & 0xf) << INTID_SHIFT | IRM
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_one_cpu_by_its_aff0_bit() {
        let value = sgi_to(SGI_RESCHEDULE, 0x0000_0003);
        assert_eq!(value & 0xffff, 1 << 3);
        assert_eq!((value >> 24) & 0xf, 0);
        assert_eq!(value & IRM, 0);
    }

    #[test]
    fn carries_the_upper_affinity_levels_and_the_intid() {
        let value = sgi_to(SGI_STOP, 0x0201_0307);
        assert_eq!((value >> 16) & 0xff, 0x03);
        assert_eq!((value >> 32) & 0xff, 0x01);
        assert_eq!((value >> 48) & 0xff, 0x02);
        assert_eq!((value >> 24) & 0xf, 1);
        assert_eq!(value & 0xffff, 1 << 7);
    }

    #[test]
    fn aff0_above_fifteen_selects_the_next_range() {
        let value = sgi_to(SGI_RESCHEDULE, 17);
        assert_eq!(value & 0xffff, 1 << 1);
        assert_eq!((value >> 44) & 0xf, 1);
    }

    #[test]
    fn broadcast_sets_irm_and_no_target() {
        let value = sgi_all_but_self(SGI_STOP);
        assert_eq!(value, IRM | 1 << 24);
    }
}
