//! Formato dos descritores de tabela de página do aarch64 (VMSAv8-64, grânulo de 4 KiB).

use rustos_hal::{CacheMode, PageFlags, PhysAddr, VirtAddr};

pub const ENTRIES: usize = 512;
pub const LEVELS: usize = 4;

pub const VALID: u64 = 1 << 0;
/// Nos níveis 0-2 distingue tabela (1) de bloco (0); no nível 3 é obrigatório.
pub const TABLE_OR_PAGE: u64 = 1 << 1;
const ATTR_INDEX_SHIFT: u32 = 2;
const READ_ONLY: u64 = 1 << 7;
const INNER_SHAREABLE: u64 = 0b11 << 8;
const ACCESS_FLAG: u64 = 1 << 10;
const PRIVILEGED_EXECUTE_NEVER: u64 = 1 << 53;
const USER_EXECUTE_NEVER: u64 = 1 << 54;
pub const OUTPUT_ADDRESS: u64 = 0x0000_FFFF_FFFF_F000;

/// Índices no MAIR_EL1. O Limine usa 0 (normal WB) e 1 (framebuffer); o 7 é do kernel.
pub const MAIR_NORMAL_INDEX: u64 = 0;
pub const MAIR_DEVICE_INDEX: u64 = 7;
/// Device-nGnRnE: sem junção, sem reordenação, sem confirmação antecipada.
pub const MAIR_DEVICE_NGNRNE: u64 = 0x00;

#[allow(
    clippy::cast_possible_truncation,
    reason = "o índice é mascarado para < 512"
)]
pub const fn table_index(virt: VirtAddr, level: usize) -> usize {
    let shift = 39 - 9 * level;
    ((virt.as_u64() >> shift) & (ENTRIES as u64 - 1)) as usize
}

pub const fn table_descriptor(next: PhysAddr) -> u64 {
    (next.as_u64() & OUTPUT_ADDRESS) | VALID | TABLE_OR_PAGE
}

/// Descritor de página do nível 3. Página do kernel nunca executa em EL0.
pub const fn page_descriptor(phys: PhysAddr, flags: PageFlags) -> u64 {
    let mut bits = (phys.as_u64() & OUTPUT_ADDRESS) | VALID | TABLE_OR_PAGE | ACCESS_FLAG;
    bits |= USER_EXECUTE_NEVER;
    bits |= match flags.cache {
        CacheMode::WriteBack => (MAIR_NORMAL_INDEX << ATTR_INDEX_SHIFT) | INNER_SHAREABLE,
        CacheMode::Device => MAIR_DEVICE_INDEX << ATTR_INDEX_SHIFT,
    };
    if !flags.writable {
        bits |= READ_ONLY;
    }
    if !flags.executable {
        bits |= PRIVILEGED_EXECUTE_NEVER;
    }
    bits
}

pub const fn is_table(entry: u64) -> bool {
    entry & (VALID | TABLE_OR_PAGE) == VALID | TABLE_OR_PAGE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_follow_the_48_bit_split() {
        let virt = VirtAddr::new(0xffff_ff80_4020_3000);
        assert_eq!(table_index(virt, 0), 511);
        assert_eq!(table_index(virt, 1), 1);
        assert_eq!(table_index(virt, 2), 1);
        assert_eq!(table_index(virt, 3), 3);
    }

    #[test]
    fn device_page_is_non_executable_with_device_attribute() {
        let entry = page_descriptor(PhysAddr::new(0x0900_0000), PageFlags::DEVICE);
        assert_eq!(entry & OUTPUT_ADDRESS, 0x0900_0000);
        assert_eq!((entry >> ATTR_INDEX_SHIFT) & 0b111, MAIR_DEVICE_INDEX);
        assert_ne!(entry & PRIVILEGED_EXECUTE_NEVER, 0);
        assert_ne!(entry & USER_EXECUTE_NEVER, 0);
        assert_ne!(entry & ACCESS_FLAG, 0);
        assert_eq!(entry & READ_ONLY, 0);
    }

    #[test]
    fn data_page_is_inner_shareable_write_back() {
        let entry = page_descriptor(PhysAddr::new(0x4000_0000), PageFlags::DATA);
        assert_eq!((entry >> ATTR_INDEX_SHIFT) & 0b111, MAIR_NORMAL_INDEX);
        assert_eq!(entry & INNER_SHAREABLE, INNER_SHAREABLE);
    }

    #[test]
    fn block_descriptor_is_not_a_table() {
        assert!(is_table(table_descriptor(PhysAddr::new(0x1000))));
        assert!(!is_table(VALID));
        assert!(!is_table(0));
    }
}
