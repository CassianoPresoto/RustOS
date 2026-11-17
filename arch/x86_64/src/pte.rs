//! Formato das entradas de tabela de página do x86_64 (paginação de 4 níveis, 4 KiB).

use rustos_hal::{CacheMode, PageFlags, PhysAddr, VirtAddr};

pub const ENTRIES: usize = 512;
pub const LEVELS: usize = 4;

pub const PRESENT: u64 = 1 << 0;
pub const WRITABLE: u64 = 1 << 1;
const WRITE_THROUGH: u64 = 1 << 3;
const CACHE_DISABLE: u64 = 1 << 4;
/// Nos níveis 1 e 2 (PDPT e PD) marca página de 1 GiB ou 2 MiB em vez de tabela.
pub const HUGE: u64 = 1 << 7;
const NO_EXECUTE: u64 = 1 << 63;
pub const OUTPUT_ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;

/// `PCD|PWT` escolhe a entrada 3 do PAT, que o Limine deixa em UC.
const UNCACHED: u64 = CACHE_DISABLE | WRITE_THROUGH;

#[allow(
    clippy::cast_possible_truncation,
    reason = "o índice é mascarado para < 512"
)]
pub const fn table_index(virt: VirtAddr, level: usize) -> usize {
    let shift = 39 - 9 * level;
    ((virt.as_u64() >> shift) & (ENTRIES as u64 - 1)) as usize
}

/// Entrada de nível intermediário: permissão ampla, quem restringe é a folha.
pub const fn table_entry(next: PhysAddr) -> u64 {
    (next.as_u64() & OUTPUT_ADDRESS) | PRESENT | WRITABLE
}

/// Entrada de página de 4 KiB (nível 1 da caminhada, a última).
pub const fn page_entry(phys: PhysAddr, flags: PageFlags) -> u64 {
    let mut bits = (phys.as_u64() & OUTPUT_ADDRESS) | PRESENT;
    if flags.writable {
        bits |= WRITABLE;
    }
    if !flags.executable {
        bits |= NO_EXECUTE;
    }
    if matches!(flags.cache, CacheMode::Device) {
        bits |= UNCACHED;
    }
    bits
}

pub const fn is_present(entry: u64) -> bool {
    entry & PRESENT != 0
}

pub const fn is_huge(entry: u64) -> bool {
    entry & HUGE != 0
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
    fn window_entries_match_the_kernel_layout() {
        assert_eq!(table_index(VirtAddr::new(0xffff_ff00_0000_0000), 0), 510);
        assert_eq!(table_index(VirtAddr::new(0xffff_fe80_0000_0000), 0), 509);
    }

    #[test]
    fn device_page_is_uncached_non_executable_and_writable() {
        let entry = page_entry(PhysAddr::new(0xFEE0_0000), PageFlags::DEVICE);
        assert_eq!(entry & OUTPUT_ADDRESS, 0xFEE0_0000);
        assert_eq!(entry & UNCACHED, UNCACHED);
        assert_ne!(entry & NO_EXECUTE, 0);
        assert_ne!(entry & WRITABLE, 0);
        assert!(is_present(entry));
    }

    #[test]
    fn data_page_is_write_back() {
        let entry = page_entry(PhysAddr::new(0x4000_0000), PageFlags::DATA);
        assert_eq!(entry & UNCACHED, 0);
        assert_ne!(entry & NO_EXECUTE, 0);
    }

    #[test]
    fn read_only_executable_page_clears_write_and_nx() {
        let flags = PageFlags {
            writable: false,
            executable: true,
            cache: CacheMode::WriteBack,
        };
        let entry = page_entry(PhysAddr::new(0x1000), flags);
        assert_eq!(entry, 0x1000 | PRESENT);
    }

    #[test]
    fn address_above_52_bits_is_masked_out() {
        let entry = page_entry(PhysAddr::new(0xFFF0_0000_0000_1000), PageFlags::DATA);
        assert_eq!(entry & OUTPUT_ADDRESS, 0x1000);
    }

    #[test]
    fn table_entry_is_present_writable_and_not_huge() {
        let entry = table_entry(PhysAddr::new(0x2000));
        assert_eq!(entry, 0x2000 | PRESENT | WRITABLE);
        assert!(!is_huge(entry));
        assert!(is_huge(HUGE | PRESENT));
    }
}
