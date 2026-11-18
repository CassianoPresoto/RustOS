//! Formato das entradas de tabela de página do x86_64 (paginação de 4 níveis, 4 KiB).

use rustos_hal::{CacheMode, PageFlags, PageFormat, PhysAddr};

pub const PRESENT: u64 = 1 << 0;
pub const WRITABLE: u64 = 1 << 1;
/// `U/S`: acessível ao anel 3; vale em todos os níveis da hierarquia.
pub const USER: u64 = 1 << 2;
const WRITE_THROUGH: u64 = 1 << 3;
const CACHE_DISABLE: u64 = 1 << 4;
const ACCESSED: u64 = 1 << 5;
const DIRTY: u64 = 1 << 6;
/// Nos níveis 1 e 2 (PDPT e PD) marca página de 1 GiB ou 2 MiB em vez de tabela.
pub const HUGE: u64 = 1 << 7;
/// Bit PAT da folha de 4 KiB (bit 7) e da de 2 MiB (bit 12).
const PAT_PAGE: u64 = 1 << 7;
const PAT_LARGE: u64 = 1 << 12;
const NO_EXECUTE: u64 = 1 << 63;
pub const OUTPUT_ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;
/// Endereço de uma folha de 2 MiB: os bits 12 a 20 são o PAT e reservados.
pub const LARGE_OUTPUT_ADDRESS: u64 = 0x000F_FFFF_FFE0_0000;

/// `PCD|PWT` escolhe a entrada 3 do PAT, que o Limine deixa em UC.
const UNCACHED: u64 = CACHE_DISABLE | WRITE_THROUGH;
/// Entrada 5 do PAT (`PAT|PWT`): o Limine a deixa em write-combining.
pub const PAT_WRITE_COMBINING_INDEX: u64 = 5;
/// Tipo de memória do PAT que o `IA32_PAT` guarda para write-combining (SDM vol. 3A, 11.12.2).
pub const PAT_TYPE_WRITE_COMBINING: u8 = 0x01;

/// Entrada de nível intermediário: permissão ampla, quem restringe é a folha.
pub const fn table_entry(next: PhysAddr) -> u64 {
    (next.as_u64() & OUTPUT_ADDRESS) | PRESENT | WRITABLE
}

/// Entrada de nível intermediário da hierarquia de uma folha de usuário: o `U/S` é o E dos níveis.
pub const fn user_table_entry(next: PhysAddr) -> u64 {
    table_entry(next) | USER
}

/// Bits comuns às folhas: permissão e acesso. `pat` é o bit PAT da folha (7 ou 12).
const fn leaf_bits(flags: PageFlags, pat: u64) -> u64 {
    let mut bits = PRESENT | ACCESSED;
    if flags.writable {
        bits |= WRITABLE | DIRTY;
    }
    if !flags.executable {
        bits |= NO_EXECUTE;
    }
    if flags.user {
        bits |= USER;
    }
    bits |= match flags.cache {
        CacheMode::WriteBack => 0,
        CacheMode::WriteCombining => pat | WRITE_THROUGH,
        CacheMode::Device | CacheMode::Other => UNCACHED,
    };
    bits
}

/// Entrada de página de 4 KiB (nível 1 da caminhada, a última).
pub const fn page_entry(phys: PhysAddr, flags: PageFlags) -> u64 {
    (phys.as_u64() & OUTPUT_ADDRESS) | leaf_bits(flags, PAT_PAGE)
}

/// Entrada de página de 2 MiB (nível 2, com `HUGE`).
pub const fn large_entry(phys: PhysAddr, flags: PageFlags) -> u64 {
    (phys.as_u64() & LARGE_OUTPUT_ADDRESS) | HUGE | leaf_bits(flags, PAT_LARGE)
}

/// Lê permissão e cache de uma folha. `large` diz onde fica o bit PAT e o endereço.
pub const fn decode_leaf(entry: u64, large: bool) -> (PhysAddr, PageFlags) {
    let (pat_bit, mask) = if large {
        (PAT_LARGE, LARGE_OUTPUT_ADDRESS)
    } else {
        (PAT_PAGE, OUTPUT_ADDRESS)
    };
    let index = ((entry & pat_bit != 0) as u64) << 2
        | ((entry & CACHE_DISABLE != 0) as u64) << 1
        | (entry & WRITE_THROUGH != 0) as u64;
    let cache = match index {
        0 => CacheMode::WriteBack,
        PAT_WRITE_COMBINING_INDEX => CacheMode::WriteCombining,
        3 => CacheMode::Device,
        _ => CacheMode::Other,
    };
    let flags = PageFlags {
        writable: entry & WRITABLE != 0,
        executable: entry & NO_EXECUTE == 0,
        user: entry & USER != 0,
        cache,
    };
    (PhysAddr::new(entry & mask), flags)
}

/// Tipo de memória que a entrada `index` (0 a 7) do `IA32_PAT` guarda.
pub const fn pat_type(pat: u64, index: u64) -> u8 {
    (pat >> (index * 8) & 0xff) as u8
}

/// Permissão efetiva ao descer: escrita é o E dos níveis, NX é o OU.
pub const fn restrict(writable: bool, no_execute: bool, entry: u64) -> (bool, bool) {
    (
        writable && entry & WRITABLE != 0,
        no_execute || entry & NO_EXECUTE != 0,
    )
}

/// Endereço virtual canônico da entrada `index` do nível mais alto mais o que vem abaixo.
pub const fn canonical(virt: u64) -> u64 {
    if virt & (1 << 47) != 0 {
        virt | 0xffff_0000_0000_0000
    } else {
        virt
    }
}

pub const fn is_present(entry: u64) -> bool {
    entry & PRESENT != 0
}

pub const fn is_huge(entry: u64) -> bool {
    entry & HUGE != 0
}

/// Formato das entradas do x86_64, para a caminhada genérica de `core/mm`.
#[derive(Debug)]
pub struct X86Format;

impl PageFormat for X86Format {
    fn table_entry(next: PhysAddr) -> u64 {
        table_entry(next)
    }

    fn user_table_entry(next: PhysAddr) -> u64 {
        user_table_entry(next)
    }

    fn page_entry(phys: PhysAddr, flags: PageFlags) -> u64 {
        page_entry(phys, flags)
    }

    fn large_entry(phys: PhysAddr, flags: PageFlags) -> u64 {
        large_entry(phys, flags)
    }

    fn is_valid(entry: u64) -> bool {
        is_present(entry)
    }

    fn is_table(entry: u64, level: usize) -> bool {
        level == 0 || !is_huge(entry)
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
        canonical(virt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(entry & (UNCACHED | PAT_PAGE), 0);
        assert_ne!(entry & NO_EXECUTE, 0);
    }

    #[test]
    fn read_only_executable_page_clears_write_and_nx() {
        let flags = PageFlags {
            writable: false,
            executable: true,
            user: false,
            cache: CacheMode::WriteBack,
        };
        let entry = page_entry(PhysAddr::new(0x1000), flags);
        assert_eq!(entry, 0x1000 | PRESENT | ACCESSED);
    }

    #[test]
    fn section_flags_encode_the_permissions_they_name() {
        let encode = |flags| page_entry(PhysAddr::new(0x1000), flags);
        let code = encode(PageFlags::CODE);
        assert_eq!(code & (WRITABLE | NO_EXECUTE), 0);
        let rodata = encode(PageFlags::RODATA);
        assert_eq!((rodata & WRITABLE, rodata & NO_EXECUTE), (0, NO_EXECUTE));
        let data = encode(PageFlags::DATA);
        assert_eq!((data & WRITABLE, data & NO_EXECUTE), (WRITABLE, NO_EXECUTE));
    }

    #[test]
    fn framebuffer_page_selects_pat_entry_5() {
        let entry = page_entry(PhysAddr::new(0xFD00_0000), PageFlags::FRAMEBUFFER);
        assert_eq!(
            entry & (PAT_PAGE | CACHE_DISABLE | WRITE_THROUGH),
            PAT_PAGE | WRITE_THROUGH
        );
        assert_ne!(entry & NO_EXECUTE, 0);
        assert_ne!(entry & WRITABLE, 0);
    }

    #[test]
    fn large_entry_is_huge_with_the_pat_bit_in_bit_12() {
        let data = large_entry(PhysAddr::new(0x4020_0000), PageFlags::DATA);
        assert_eq!(data & LARGE_OUTPUT_ADDRESS, 0x4020_0000);
        assert!(is_huge(data));
        assert_eq!(data & PAT_LARGE, 0);
        let fb = large_entry(PhysAddr::new(0x4020_0000), PageFlags::FRAMEBUFFER);
        assert_eq!(fb & PAT_LARGE, PAT_LARGE);
        assert_eq!(fb & (CACHE_DISABLE | WRITE_THROUGH), WRITE_THROUGH);
    }

    #[test]
    fn large_entry_masks_address_bits_below_2_mib() {
        let entry = large_entry(PhysAddr::new(0x4020_1000), PageFlags::DATA);
        assert_eq!(entry & LARGE_OUTPUT_ADDRESS, 0x4020_0000);
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
            assert_eq!(decode_leaf(page_entry(phys, flags), false), (phys, flags));
            let big = PhysAddr::new(0x4020_0000);
            assert_eq!(decode_leaf(large_entry(big, flags), true), (big, flags));
        }
    }

    #[test]
    fn unknown_pat_index_decodes_as_other() {
        let entry = PRESENT | CACHE_DISABLE;
        assert_eq!(decode_leaf(entry, false).1.cache, CacheMode::Other);
    }

    #[test]
    fn restrict_ands_write_and_ors_no_execute() {
        assert_eq!(restrict(true, false, WRITABLE), (true, false));
        assert_eq!(restrict(true, false, 0), (false, false));
        assert_eq!(restrict(true, false, NO_EXECUTE | WRITABLE), (true, true));
        assert_eq!(restrict(false, true, WRITABLE), (false, true));
    }

    #[test]
    fn pat_type_reads_one_byte_per_entry() {
        let limine = 0x0000_0105_0007_0406_u64;
        assert_eq!(pat_type(limine, 0), 0x06);
        assert_eq!(pat_type(limine, 5), PAT_TYPE_WRITE_COMBINING);
        assert_eq!(pat_type(limine, 3), 0x00);
    }

    #[test]
    fn canonical_extends_bit_47() {
        assert_eq!(canonical(255 << 39), 255 << 39);
        assert_eq!(canonical(256 << 39), 0xffff_8000_0000_0000);
        assert_eq!(canonical(511 << 39), 0xffff_ff80_0000_0000);
    }

    #[test]
    fn user_pages_set_the_user_bit_and_keep_the_other_permissions() {
        let encode = |flags| page_entry(PhysAddr::new(0x1000), flags);
        let code = encode(PageFlags::USER_CODE);
        assert_eq!((code & USER, code & (WRITABLE | NO_EXECUTE)), (USER, 0));
        let data = encode(PageFlags::USER_DATA);
        assert_eq!(
            data & (USER | WRITABLE | NO_EXECUTE),
            USER | WRITABLE | NO_EXECUTE
        );
        assert_eq!(
            encode(PageFlags::DATA) & USER,
            0,
            "página do kernel nunca é de usuário"
        );
        assert!(decode_leaf(data, false).1.user);
        assert!(!decode_leaf(encode(PageFlags::DATA), false).1.user);
    }

    #[test]
    fn user_tables_carry_the_user_bit_and_kernel_tables_do_not() {
        assert_eq!(
            user_table_entry(PhysAddr::new(0x2000)),
            0x2000 | PRESENT | WRITABLE | USER
        );
        assert_eq!(table_entry(PhysAddr::new(0x2000)) & USER, 0);
        assert_eq!(
            X86Format::user_table_entry(PhysAddr::new(0x2000)),
            user_table_entry(PhysAddr::new(0x2000))
        );
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

    #[test]
    fn the_format_port_tells_tables_from_huge_leaves_and_empty_entries() {
        let table = X86Format::table_entry(PhysAddr::new(0x2000));
        let large = X86Format::large_entry(PhysAddr::new(0x4020_0000), PageFlags::DATA);
        let small = X86Format::page_entry(PhysAddr::new(0x1000), PageFlags::DATA);
        assert!(X86Format::is_valid(table) && X86Format::is_valid(small));
        assert!(!X86Format::is_valid(0));
        assert!(X86Format::is_table(table, 1) && X86Format::is_table(table, 2));
        assert!(!X86Format::is_table(large, 2));
        assert!(!X86Format::is_table(HUGE | PRESENT, 1));
        assert_eq!(X86Format::table_address(table), PhysAddr::new(0x2000));
    }
}
