//! Formato dos descritores de tabela de página do aarch64 (VMSAv8-64, grânulo de 4 KiB).

use rustos_hal::{CacheMode, PageFlags, PageFormat, PhysAddr};

pub const VALID: u64 = 1 << 0;
/// Nos níveis 0-2 distingue tabela (1) de bloco (0); no nível 3 é obrigatório.
pub const TABLE_OR_PAGE: u64 = 1 << 1;
const ATTR_INDEX_SHIFT: u32 = 2;
/// `AP[1]`: a página é acessível a EL0 (`PTE_USER` do Linux).
const USER_ACCESS: u64 = 1 << 6;
const READ_ONLY: u64 = 1 << 7;
/// `nG`: a tradução é por ASID, não global; toda página de usuário a liga.
const NOT_GLOBAL: u64 = 1 << 11;
const INNER_SHAREABLE: u64 = 0b11 << 8;
const OUTER_SHAREABLE: u64 = 0b10 << 8;
const SHAREABILITY_MASK: u64 = 0b11 << 8;
const ACCESS_FLAG: u64 = 1 << 10;
const PRIVILEGED_EXECUTE_NEVER: u64 = 1 << 53;
const USER_EXECUTE_NEVER: u64 = 1 << 54;
pub const OUTPUT_ADDRESS: u64 = 0x0000_FFFF_FFFF_F000;
/// Endereço de um bloco de 2 MiB (nível 2): os bits 12 a 20 são reservados.
pub const BLOCK_ADDRESS: u64 = 0x0000_FFFF_FFE0_0000;
/// Restrições herdadas que um descritor de tabela pode impor aos de baixo.
const TABLE_AP_NO_WRITE: u64 = 1 << 62;
const TABLE_PXN: u64 = 1 << 59;

/// Índices no MAIR_EL1. O Limine usa 0 (normal WB) e 1 (framebuffer); o 7 é do kernel.
pub const MAIR_NORMAL_INDEX: u64 = 0;
pub const MAIR_FRAMEBUFFER_INDEX: u64 = 1;
pub const MAIR_DEVICE_INDEX: u64 = 7;
/// Device-nGnRnE: sem junção, sem reordenação, sem confirmação antecipada.
pub const MAIR_DEVICE_NGNRNE: u64 = 0x00;

pub const fn table_descriptor(next: PhysAddr) -> u64 {
    (next.as_u64() & OUTPUT_ADDRESS) | VALID | TABLE_OR_PAGE
}

/// Atributos comuns a página e bloco. Página do kernel nunca executa em EL0. Página de usuário
/// nunca executa em EL1 (`PXN`) e só executa em EL0 se `executable`.
const fn leaf_bits(flags: PageFlags) -> u64 {
    if flags.user {
        return user_leaf_bits(flags);
    }
    let mut bits = VALID | ACCESS_FLAG | USER_EXECUTE_NEVER;
    bits |= match flags.cache {
        CacheMode::WriteBack => (MAIR_NORMAL_INDEX << ATTR_INDEX_SHIFT) | INNER_SHAREABLE,
        CacheMode::WriteCombining => (MAIR_FRAMEBUFFER_INDEX << ATTR_INDEX_SHIFT) | OUTER_SHAREABLE,
        CacheMode::Device | CacheMode::Other => MAIR_DEVICE_INDEX << ATTR_INDEX_SHIFT,
    };
    if !flags.writable {
        bits |= READ_ONLY;
    }
    if !flags.executable {
        bits |= PRIVILEGED_EXECUTE_NEVER;
    }
    bits
}

const fn user_leaf_bits(flags: PageFlags) -> u64 {
    let mut bits = VALID | ACCESS_FLAG | USER_ACCESS | NOT_GLOBAL | PRIVILEGED_EXECUTE_NEVER;
    // O mesmo atributo que o alias do kernel usa: o framebuffer é o mesmo endereço nos dois.
    bits |= match flags.cache {
        CacheMode::WriteBack => (MAIR_NORMAL_INDEX << ATTR_INDEX_SHIFT) | INNER_SHAREABLE,
        CacheMode::WriteCombining => (MAIR_FRAMEBUFFER_INDEX << ATTR_INDEX_SHIFT) | OUTER_SHAREABLE,
        CacheMode::Device | CacheMode::Other => MAIR_DEVICE_INDEX << ATTR_INDEX_SHIFT,
    };
    if !flags.writable {
        bits |= READ_ONLY;
    }
    if !flags.executable {
        bits |= USER_EXECUTE_NEVER;
    }
    bits
}

/// Descritor de página do nível 3.
pub const fn page_descriptor(phys: PhysAddr, flags: PageFlags) -> u64 {
    (phys.as_u64() & OUTPUT_ADDRESS) | TABLE_OR_PAGE | leaf_bits(flags)
}

/// Descritor de bloco de 2 MiB do nível 2 (o bit 1 zerado o distingue de tabela).
pub const fn block_descriptor(phys: PhysAddr, flags: PageFlags) -> u64 {
    (phys.as_u64() & BLOCK_ADDRESS) | leaf_bits(flags)
}

/// Lê permissão e cache de uma folha. `block` diz onde termina o endereço.
pub const fn decode_leaf(entry: u64, block: bool) -> (PhysAddr, PageFlags) {
    let index = entry >> ATTR_INDEX_SHIFT & 0b111;
    let cache = match index {
        MAIR_NORMAL_INDEX if entry & SHAREABILITY_MASK == INNER_SHAREABLE => CacheMode::WriteBack,
        MAIR_FRAMEBUFFER_INDEX => CacheMode::WriteCombining,
        MAIR_DEVICE_INDEX => CacheMode::Device,
        _ => CacheMode::Other,
    };
    let user = entry & USER_ACCESS != 0;
    let flags = PageFlags {
        writable: entry & READ_ONLY == 0,
        executable: if user {
            entry & USER_EXECUTE_NEVER == 0
        } else {
            entry & PRIVILEGED_EXECUTE_NEVER == 0
        },
        user,
        cache,
    };
    let mask = if block { BLOCK_ADDRESS } else { OUTPUT_ADDRESS };
    (PhysAddr::new(entry & mask), flags)
}

/// Permissão efetiva ao descer por um descritor de tabela: restrição só se soma.
pub const fn restrict(writable: bool, no_execute: bool, table: u64) -> (bool, bool) {
    (
        writable && table & TABLE_AP_NO_WRITE == 0,
        no_execute || table & TABLE_PXN != 0,
    )
}

/// Endereço virtual da metade alta (`TTBR1_EL1`) a partir do que as tabelas resolvem.
pub const fn upper_half(virt: u64) -> u64 {
    virt | 0xffff_0000_0000_0000
}

pub const fn is_table(entry: u64) -> bool {
    entry & (VALID | TABLE_OR_PAGE) == VALID | TABLE_OR_PAGE
}

/// Formato dos descritores do aarch64, para a caminhada genérica de `core/mm`.
#[derive(Debug)]
pub struct Aarch64Format;

impl PageFormat for Aarch64Format {
    fn table_entry(next: PhysAddr) -> u64 {
        table_descriptor(next)
    }

    fn page_entry(phys: PhysAddr, flags: PageFlags) -> u64 {
        page_descriptor(phys, flags)
    }

    fn large_entry(phys: PhysAddr, flags: PageFlags) -> u64 {
        block_descriptor(phys, flags)
    }

    fn is_valid(entry: u64) -> bool {
        entry & VALID != 0
    }

    fn is_table(entry: u64, _level: usize) -> bool {
        is_table(entry)
    }

    fn table_address(entry: u64) -> PhysAddr {
        PhysAddr::new(entry & OUTPUT_ADDRESS)
    }

    fn decode_leaf(entry: u64, large: bool) -> (PhysAddr, PageFlags) {
        decode_leaf(entry, large)
    }

    fn restrict(writable: bool, no_execute: bool, entry: u64) -> (bool, bool) {
        restrict(writable, no_execute, entry)
    }

    fn canonical(virt: u64) -> u64 {
        upper_half(virt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_2_mib_device_block_is_a_leaf_that_is_not_a_table() {
        let entry = block_descriptor(PhysAddr::new(0x0900_0000), PageFlags::DEVICE);
        assert_eq!(entry & BLOCK_ADDRESS, 0x0900_0000);
        assert!(!is_table(entry));
        assert_eq!((entry >> ATTR_INDEX_SHIFT) & 0b111, MAIR_DEVICE_INDEX);
        assert_ne!(entry & PRIVILEGED_EXECUTE_NEVER, 0);
        assert_eq!(0x0900_0000 % (1u64 << 21), 0);
    }

    #[test]
    fn user_framebuffer_page_is_write_combining_like_the_kernel_alias() {
        let user = PageFlags {
            cache: CacheMode::WriteCombining,
            ..PageFlags::USER_DATA
        };
        let entry = page_descriptor(PhysAddr::new(0x0900_0000), user);
        assert_eq!((entry >> ATTR_INDEX_SHIFT) & 0b111, MAIR_FRAMEBUFFER_INDEX);
        assert_eq!(entry & SHAREABILITY_MASK, OUTER_SHAREABLE);
        let (phys, flags) = decode_leaf(entry, false);
        assert_eq!(phys, PhysAddr::new(0x0900_0000));
        assert_eq!(flags, user);
    }

    #[test]
    fn data_page_is_inner_shareable_write_back() {
        let entry = page_descriptor(PhysAddr::new(0x4000_0000), PageFlags::DATA);
        assert_eq!((entry >> ATTR_INDEX_SHIFT) & 0b111, MAIR_NORMAL_INDEX);
        assert_eq!(entry & INNER_SHAREABLE, INNER_SHAREABLE);
    }

    #[test]
    fn section_flags_encode_the_permissions_they_name() {
        let encode = |flags| page_descriptor(PhysAddr::new(0x4000_0000), flags);
        let code = encode(PageFlags::CODE);
        assert_eq!(code & (READ_ONLY | PRIVILEGED_EXECUTE_NEVER), READ_ONLY);
        let rodata = encode(PageFlags::RODATA);
        assert_eq!(
            rodata & (READ_ONLY | PRIVILEGED_EXECUTE_NEVER),
            READ_ONLY | PRIVILEGED_EXECUTE_NEVER
        );
        let data = encode(PageFlags::DATA);
        assert_eq!(
            data & (READ_ONLY | PRIVILEGED_EXECUTE_NEVER),
            PRIVILEGED_EXECUTE_NEVER
        );
    }

    #[test]
    fn framebuffer_uses_the_attribute_the_bootloader_left_at_index_1() {
        let entry = page_descriptor(PhysAddr::new(0x4000_0000), PageFlags::FRAMEBUFFER);
        assert_eq!((entry >> ATTR_INDEX_SHIFT) & 0b111, MAIR_FRAMEBUFFER_INDEX);
        assert_eq!(entry & SHAREABILITY_MASK, OUTER_SHAREABLE);
        assert_ne!(entry & PRIVILEGED_EXECUTE_NEVER, 0);
    }

    #[test]
    fn block_descriptor_has_no_table_bit_and_masks_the_low_address() {
        let entry = block_descriptor(PhysAddr::new(0x4020_1000), PageFlags::DATA);
        assert_eq!(entry & (VALID | TABLE_OR_PAGE), VALID);
        assert_eq!(entry & BLOCK_ADDRESS, 0x4020_0000);
        assert!(!is_table(entry));
        assert_ne!(entry & ACCESS_FLAG, 0);
    }

    #[test]
    fn decode_returns_what_encode_wrote() {
        for flags in [
            PageFlags::CODE,
            PageFlags::RODATA,
            PageFlags::DATA,
            PageFlags::DEVICE,
            PageFlags::FRAMEBUFFER,
        ] {
            let phys = PhysAddr::new(0x4000_1000);
            assert_eq!(
                decode_leaf(page_descriptor(phys, flags), false),
                (phys, flags)
            );
            let big = PhysAddr::new(0x4020_0000);
            assert_eq!(
                decode_leaf(block_descriptor(big, flags), true),
                (big, flags)
            );
        }
    }

    #[test]
    fn unknown_attribute_index_decodes_as_other() {
        let entry = VALID | TABLE_OR_PAGE | (3 << ATTR_INDEX_SHIFT);
        assert_eq!(decode_leaf(entry, false).1.cache, CacheMode::Other);
    }

    #[test]
    fn table_restrictions_only_accumulate() {
        assert_eq!(
            restrict(true, false, table_descriptor(PhysAddr::new(0x1000))),
            (true, false)
        );
        assert_eq!(restrict(true, false, TABLE_AP_NO_WRITE), (false, false));
        assert_eq!(restrict(true, false, TABLE_PXN), (true, true));
        assert_eq!(restrict(false, true, 0), (false, true));
    }

    #[test]
    fn upper_half_fills_the_top_16_bits() {
        assert_eq!(upper_half(0x0000_ff80_0000_0000), 0xffff_ff80_0000_0000);
    }

    #[test]
    fn block_descriptor_is_not_a_table() {
        assert!(is_table(table_descriptor(PhysAddr::new(0x1000))));
        assert!(!is_table(VALID));
        assert!(!is_table(0));
    }

    #[test]
    fn the_format_port_tells_tables_from_blocks_and_empty_entries() {
        let table = Aarch64Format::table_entry(PhysAddr::new(0x2000));
        let block = Aarch64Format::large_entry(PhysAddr::new(0x4020_0000), PageFlags::DATA);
        let page = Aarch64Format::page_entry(PhysAddr::new(0x1000), PageFlags::DATA);
        assert!(Aarch64Format::is_valid(table) && Aarch64Format::is_valid(block));
        assert!(!Aarch64Format::is_valid(0));
        assert!(Aarch64Format::is_table(table, 1));
        assert!(!Aarch64Format::is_table(block, 2));
        assert!(!Aarch64Format::is_table(VALID, 1));
        assert!(Aarch64Format::is_valid(page));
        assert_eq!(Aarch64Format::table_address(table), PhysAddr::new(0x2000));
    }

    #[test]
    fn user_code_is_executable_only_by_el0_and_read_only() {
        let entry = page_descriptor(PhysAddr::new(0x4000_0000), PageFlags::USER_CODE);
        assert_ne!(entry & USER_ACCESS, 0);
        assert_ne!(entry & NOT_GLOBAL, 0);
        assert_ne!(entry & READ_ONLY, 0);
        assert_ne!(
            entry & PRIVILEGED_EXECUTE_NEVER,
            0,
            "EL1 nunca executa página de usuário"
        );
        assert_eq!(entry & USER_EXECUTE_NEVER, 0);
        assert!(decode_leaf(entry, false).1.executable);
    }

    #[test]
    fn user_data_never_executes_and_keeps_write() {
        let entry = page_descriptor(PhysAddr::new(0x4000_0000), PageFlags::USER_DATA);
        assert_eq!(entry & READ_ONLY, 0);
        assert_ne!(entry & USER_EXECUTE_NEVER, 0);
        assert_ne!(entry & PRIVILEGED_EXECUTE_NEVER, 0);
        let (_, flags) = decode_leaf(entry, false);
        assert_eq!(flags, PageFlags::USER_DATA);
        let rodata = decode_leaf(
            page_descriptor(PhysAddr::new(0x1000), PageFlags::USER_RODATA),
            false,
        )
        .1;
        assert_eq!(rodata, PageFlags::USER_RODATA);
    }

    #[test]
    fn kernel_pages_are_never_el0_accessible_or_non_global() {
        for flags in [
            PageFlags::CODE,
            PageFlags::RODATA,
            PageFlags::DATA,
            PageFlags::DEVICE,
        ] {
            let entry = page_descriptor(PhysAddr::new(0x1000), flags);
            assert_eq!(entry & (USER_ACCESS | NOT_GLOBAL), 0);
            assert_ne!(entry & USER_EXECUTE_NEVER, 0);
            assert!(!decode_leaf(entry, false).1.user);
        }
    }
}
