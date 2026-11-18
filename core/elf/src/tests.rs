//! Testes do validador ELF64: um ELF sintético montado aqui com um teste de
//! recusa por regra do SDD, e dois ELF reais (x86_64 e aarch64).

use super::*;

const PAGE: u64 = 4096;
const USER_MIN: u64 = 0x1_0000;
const USER_TOP: u64 = 0x8000_0000_0000;

fn limits() -> Limits {
    Limits {
        machine: EM_X86_64,
        user_min: USER_MIN,
        user_top: USER_TOP,
        page_size: PAGE,
        max_loads_bytes: 64 << 20,
    }
}

#[derive(Clone, Copy)]
struct ProgramHeader {
    kind: u32,
    flags: u32,
    offset: u64,
    vaddr: u64,
    filesz: u64,
    memsz: u64,
    align: u64,
}

fn load(flags: u32, offset: u64, vaddr: u64, filesz: u64, memsz: u64) -> ProgramHeader {
    ProgramHeader {
        kind: PT_LOAD,
        flags,
        offset,
        vaddr,
        filesz,
        memsz,
        align: PAGE,
    }
}

/// O ELF que o teste dobra: cada campo é escrito como está, válido ou não.
#[derive(Clone)]
struct Elf {
    ident: [u8; 16],
    etype: u16,
    machine: u16,
    version: u32,
    entry: u64,
    phoff: u64,
    phentsize: u16,
    phnum: Option<u16>,
    phdrs: Vec<ProgramHeader>,
    len: usize,
}

fn base() -> Elf {
    let mut ident = [0u8; 16];
    ident[..4].copy_from_slice(b"\x7fELF");
    ident[4] = 2;
    ident[5] = 1;
    ident[6] = 1;
    Elf {
        ident,
        etype: ET_EXEC,
        machine: EM_X86_64,
        version: 1,
        entry: 0x40_1000,
        phoff: 64,
        phentsize: 56,
        phnum: None,
        phdrs: vec![
            load(PF_R | PF_X, 0, 0x40_0000, 0x1010, 0x1010),
            load(PF_R, 0x2000, 0x40_2000, 0x20, 0x20),
            load(PF_R | PF_W, 0x3000, 0x40_3000, 0x10, 0x2000),
        ],
        len: 0x4010,
    }
}

impl Elf {
    fn build(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.len];
        let mut put = |at: usize, bytes: &[u8]| {
            if at + bytes.len() <= out.len() {
                out[at..at + bytes.len()].copy_from_slice(bytes);
            }
        };
        put(0, &self.ident);
        put(16, &self.etype.to_le_bytes());
        put(18, &self.machine.to_le_bytes());
        put(20, &self.version.to_le_bytes());
        put(24, &self.entry.to_le_bytes());
        put(32, &self.phoff.to_le_bytes());
        put(52, &64u16.to_le_bytes());
        put(54, &self.phentsize.to_le_bytes());
        let phnum = self
            .phnum
            .unwrap_or(u16::try_from(self.phdrs.len()).unwrap());
        put(56, &phnum.to_le_bytes());
        for (index, phdr) in self.phdrs.iter().enumerate() {
            let at = 64 + index * 56;
            put(at, &phdr.kind.to_le_bytes());
            put(at + 4, &phdr.flags.to_le_bytes());
            put(at + 8, &phdr.offset.to_le_bytes());
            put(at + 16, &phdr.vaddr.to_le_bytes());
            put(at + 24, &phdr.vaddr.to_le_bytes());
            put(at + 32, &phdr.filesz.to_le_bytes());
            put(at + 40, &phdr.memsz.to_le_bytes());
            put(at + 48, &phdr.align.to_le_bytes());
        }
        out
    }

    fn parse(&self) -> Result<Image, Error> {
        parse(&self.build(), &limits())
    }
}

fn refused(elf: &Elf) -> Error {
    elf.parse().expect_err("devia recusar")
}

fn with_load(index: usize, change: impl FnOnce(&mut ProgramHeader)) -> Elf {
    let mut elf = base();
    change(&mut elf.phdrs[index]);
    elf
}

fn with_extra(kind: u32) -> Elf {
    let mut elf = base();
    elf.phdrs.push(ProgramHeader {
        kind,
        flags: 0,
        offset: 0,
        vaddr: 0,
        filesz: 0,
        memsz: 0,
        align: 0,
    });
    elf
}

#[test]
fn valid_elf_is_accepted() {
    let image = base().parse().unwrap();
    assert_eq!(image.entry, 0x40_1000);
    assert_eq!(image.loads().len(), 3);
    assert_eq!(image.loads()[2].memsz, 0x2000);
    assert_eq!(image.phnum, 3);
    assert!(image.loads()[0].executable() && !image.loads()[0].writable());
    assert!(image.loads()[2].writable() && !image.loads()[2].executable());
}

#[test]
fn phdr_address_comes_from_the_load_that_holds_the_table() {
    assert_eq!(base().parse().unwrap().phdr, Some(0x40_0040));
    let mut elf = base();
    elf.phdrs[0] = load(PF_R | PF_X, 0x1000, 0x40_1000, 0x10, 0x10);
    elf.entry = 0x40_1000;
    assert_eq!(elf.parse().unwrap().phdr, None);
}

#[test]
fn phdr_table_half_inside_a_load_gives_none() {
    let mut elf = base();
    elf.phdrs[0] = load(PF_R | PF_X, 0, 0x40_0000, 0x80, 0x80);
    elf.entry = 0x40_0000;
    assert_eq!(elf.parse().unwrap().phdr, None);
}

#[test]
fn file_bytes_slices_the_segment() {
    let mut bytes = base().build();
    bytes[0x3000] = 0xab;
    let image = parse(&bytes, &limits()).unwrap();
    let data = image.loads()[2].file_bytes(&bytes).unwrap();
    assert_eq!(data.len(), 0x10);
    assert_eq!(data[0], 0xab);
    assert_eq!(image.loads()[2].file_bytes(&bytes[..0x3008]), None);
}

#[test]
fn refuses_bad_magic() {
    for index in 0..4 {
        let mut elf = base();
        elf.ident[index] ^= 0xff;
        assert_eq!(refused(&elf), Error::BadMagic);
    }
}

#[test]
fn refuses_wrong_class_and_data() {
    for value in [0, 1, 3] {
        let mut elf = base();
        elf.ident[4] = value;
        assert_eq!(refused(&elf), Error::BadClass);
    }
    for value in [0, 2] {
        let mut elf = base();
        elf.ident[5] = value;
        assert_eq!(refused(&elf), Error::BadData);
    }
}

#[test]
fn refuses_wrong_version() {
    let mut elf = base();
    elf.ident[6] = 0;
    assert_eq!(refused(&elf), Error::BadVersion);
    let mut elf = base();
    elf.version = 0;
    assert_eq!(refused(&elf), Error::BadVersion);
    elf.version = 2;
    assert_eq!(refused(&elf), Error::BadVersion);
}

#[test]
fn refuses_non_zero_osabi() {
    for value in [1, 3, 9, 255] {
        let mut elf = base();
        elf.ident[7] = value;
        assert_eq!(refused(&elf), Error::BadOsAbi);
    }
}

#[test]
fn refuses_every_type_but_exec() {
    for etype in [0, 1, 3, 4, 0xfe00, 0xffff] {
        let mut elf = base();
        elf.etype = etype;
        assert_eq!(refused(&elf), Error::BadType, "{etype}");
    }
}

#[test]
fn refuses_machine_that_is_not_the_kernels() {
    let mut elf = base();
    elf.machine = EM_AARCH64;
    assert_eq!(refused(&elf), Error::BadMachine);
    elf.machine = 0;
    assert_eq!(refused(&elf), Error::BadMachine);
    let mut other = limits();
    other.machine = EM_AARCH64;
    assert_eq!(parse(&base().build(), &other), Err(Error::BadMachine));
    elf.machine = EM_AARCH64;
    assert!(parse(&elf.build(), &other).is_ok());
}

#[test]
fn refuses_wrong_phentsize() {
    for size in [0, 1, 55, 57, 64, 0xffff] {
        let mut elf = base();
        elf.phentsize = size;
        assert_eq!(refused(&elf), Error::BadPhentsize, "{size}");
    }
}

#[test]
fn refuses_phdr_table_of_zero_or_too_many_bytes() {
    let mut elf = base();
    elf.phnum = Some(0);
    assert_eq!(refused(&elf), Error::BadPhdrTableSize);
    for phnum in [1171, 5000, u16::MAX] {
        elf.phnum = Some(phnum);
        assert_eq!(refused(&elf), Error::BadPhdrTableSize, "{phnum}");
    }
}

#[test]
fn phdr_table_of_exactly_the_maximum_is_accepted() {
    let mut elf = base();
    elf.phnum = Some(1170);
    elf.len = 0x2_0000;
    assert_eq!(elf.parse().unwrap().loads().len(), 3);
}

#[test]
fn refuses_phdr_table_outside_the_file() {
    let len = base().len;
    for phoff in [
        len as u64,
        len as u64 - 1,
        len as u64 - 167,
        u64::MAX,
        u64::MAX - 100,
    ] {
        let mut elf = base();
        elf.phoff = phoff;
        assert_eq!(refused(&elf), Error::PhdrTableOutOfFile, "{phoff:#x}");
    }
    let mut elf = base();
    elf.phnum = Some(1170);
    assert_eq!(refused(&elf), Error::PhdrTableOutOfFile);
}

#[test]
fn phdr_table_ending_exactly_at_the_file_end_is_read() {
    let mut elf = base();
    elf.phoff = (elf.len - 168) as u64;
    assert_ne!(refused(&elf), Error::PhdrTableOutOfFile);
}

#[test]
fn refuses_interp_tls_and_dynamic() {
    assert_eq!(refused(&with_extra(PT_INTERP)), Error::Interp);
    assert_eq!(refused(&with_extra(PT_TLS)), Error::Tls);
    assert_eq!(refused(&with_extra(PT_DYNAMIC)), Error::Dynamic);
}

#[test]
fn ignores_note_phdr_gnu_stack_relro_and_unknown_types() {
    for kind in [
        4,
        6,
        0x6474_e550,
        0x6474_e551,
        0x6474_e552,
        0x1234_5678,
        0x7000_0000,
    ] {
        assert!(with_extra(kind).parse().is_ok(), "{kind:#x}");
    }
}

#[test]
fn ignored_types_are_not_validated() {
    let mut elf = with_extra(0x6474_e551);
    elf.phdrs[3] = ProgramHeader {
        kind: 0x6474_e551,
        flags: PF_R | PF_W | PF_X,
        offset: u64::MAX,
        vaddr: u64::MAX,
        filesz: u64::MAX,
        memsz: 0,
        align: 3,
    };
    assert!(elf.parse().is_ok());
}

#[test]
fn refuses_filesz_greater_than_memsz() {
    let elf = with_load(2, |p| p.filesz = p.memsz + 1);
    assert_eq!(refused(&elf), Error::FileszGreaterThanMemsz);
}

#[test]
fn refuses_align_that_is_not_a_power_of_two() {
    for align in [3, 6, 0x1800, u64::MAX, (1 << 63) + 1] {
        let elf = with_load(1, |p| p.align = align);
        assert_eq!(refused(&elf), Error::BadAlign, "{align:#x}");
    }
}

#[test]
fn accepts_align_zero_one_and_powers_of_two() {
    for align in [0, 1, 2, 16, 4096, 0x20_0000, 1 << 63] {
        let elf = with_load(1, |p| p.align = align);
        assert!(elf.parse().is_ok(), "{align:#x}");
    }
}

#[test]
fn refuses_file_range_outside_or_overflowing() {
    let len = base().len as u64;
    for (offset, filesz) in [
        (0x4000, 0x11),
        (len, 1),
        (len + 1, 0),
        (u64::MAX, 1),
        (u64::MAX - 5, 10),
        (0x3000, u64::MAX),
        (0x3ff0, 0x21),
    ] {
        let elf = with_load(2, |p| {
            p.offset = offset;
            p.filesz = filesz;
            p.memsz = filesz.max(p.memsz);
            p.vaddr = 0x40_3000 + (offset & 0xfff);
        });
        assert_eq!(
            refused(&elf),
            Error::SegmentOutOfFile,
            "{offset:#x}+{filesz:#x}"
        );
    }
}

#[test]
fn file_range_ending_at_the_last_byte_is_accepted() {
    let elf = with_load(2, |p| {
        p.offset = 0x4000;
        p.vaddr = 0x40_3000;
        p.filesz = 0x10;
    });
    assert!(elf.parse().is_ok());
}

#[test]
fn refuses_empty_load() {
    let elf = with_load(2, |p| {
        p.filesz = 0;
        p.memsz = 0;
    });
    assert_eq!(refused(&elf), Error::EmptyLoad);
}

#[test]
fn refuses_vaddr_overflow() {
    let cases = [
        (u64::MAX, 1, Error::VaddrOverflow),
        (u64::MAX - 5, 10, Error::VaddrOverflow),
        (1 << 63, 1 << 63, Error::VaddrOverflow),
        (u64::MAX - 0x20, 0x10, Error::VaddrOverflow),
        (u64::MAX - 0xfff, 0x10, Error::VaddrOverflow),
        (USER_TOP + 1, 0x10, Error::OutOfUserRange),
    ];
    for (vaddr, memsz, expected) in cases {
        let elf = with_load(2, |p| {
            p.vaddr = vaddr;
            p.memsz = memsz;
            p.filesz = 0x10.min(memsz);
            p.offset = 0x3000 + (vaddr & 0xfff);
        });
        assert_eq!(refused(&elf), expected, "{vaddr:#x}+{memsz:#x}");
    }
}

#[test]
fn rounding_overflow_is_its_own_refusal() {
    let elf = with_load(2, |p| {
        p.vaddr = u64::MAX - 0x20;
        p.memsz = 0x10;
        p.filesz = 0x10;
        p.offset = 0x3000 + (p.vaddr & 0xfff);
    });
    assert_eq!(refused(&elf), Error::VaddrOverflow);
}

#[test]
fn refuses_ranges_outside_the_user_window_after_rounding() {
    for (vaddr, memsz) in [
        (0, 0x10),
        (USER_MIN - 1, 0x10),
        (USER_MIN - 0x10, 0x20),
        (USER_TOP, 0x10),
        (USER_TOP - 0x10, 0x11),
        (USER_TOP - 0x1000, 0x1001),
        (USER_TOP + 0x1000, 0x10),
    ] {
        let elf = with_load(2, |p| {
            p.vaddr = vaddr;
            p.memsz = memsz;
            p.filesz = 0;
            p.offset = 0x3000 + (vaddr & 0xfff);
            p.align = PAGE;
        });
        assert_eq!(
            refused(&elf),
            Error::OutOfUserRange,
            "{vaddr:#x}+{memsz:#x}"
        );
    }
}

#[test]
fn window_edges_are_inclusive_of_the_page_range() {
    for (vaddr, memsz) in [
        (USER_MIN, 0x10),
        (USER_MIN + 0x10, 0x10),
        (USER_TOP - 0x10, 0x10),
        (USER_TOP - 0x1000, 0x1000),
    ] {
        let mut elf = base();
        elf.phdrs = vec![load(PF_R | PF_X, 0x1000, vaddr, 0x10, memsz.max(0x10))];
        elf.phdrs[0].offset = 0x1000 + (vaddr & 0xfff);
        elf.entry = vaddr;
        assert!(elf.parse().is_ok(), "{vaddr:#x}+{memsz:#x}");
    }
}

#[test]
fn refuses_vaddr_and_offset_that_differ_modulo_the_page() {
    for delta in [1, 0x10, 0xfff] {
        let elf = with_load(2, |p| p.vaddr += delta);
        assert_eq!(refused(&elf), Error::Misaligned, "{delta:#x}");
    }
}

#[test]
fn congruence_is_modulo_the_page_not_modulo_p_align() {
    let elf = with_load(2, |p| {
        p.vaddr = 0x40_3010;
        p.offset = 0x3010 - 0x10 + 0x10;
        p.filesz = 0;
        p.align = 0x20_0000;
    });
    assert!(elf.parse().is_ok());
    let elf = with_load(2, |p| {
        p.vaddr = 0x40_3010;
        p.offset = 0x3000;
        p.align = 0x20_0000;
    });
    assert_eq!(refused(&elf), Error::Misaligned);
}

#[test]
fn refuses_writable_and_executable() {
    for index in 0..3 {
        let elf = with_load(index, |p| p.flags = PF_R | PF_W | PF_X);
        assert_eq!(refused(&elf), Error::WritableAndExecutable);
    }
}

#[test]
fn refuses_load_without_read() {
    for flags in [0, PF_W, PF_X, 0xffff_fff8] {
        let elf = with_load(1, |p| p.flags = flags);
        assert_eq!(refused(&elf), Error::NotReadable, "{flags:#x}");
    }
}

#[test]
fn refuses_more_than_sixteen_loads() {
    let mut elf = base();
    elf.phdrs.clear();
    for index in 0..17u64 {
        elf.phdrs.push(load(
            PF_R | PF_X,
            0x1000,
            0x40_0000 + index * 0x1000,
            0x10,
            0x10,
        ));
    }
    elf.len = 0x2000;
    elf.entry = 0x40_0000;
    assert_eq!(refused(&elf), Error::TooManyLoads);
    elf.phdrs.pop();
    assert_eq!(elf.parse().unwrap().loads().len(), 16);
}

#[test]
fn refuses_unsorted_loads() {
    let mut elf = base();
    elf.phdrs.swap(1, 2);
    assert_eq!(refused(&elf), Error::Unsorted);
    let mut elf = base();
    elf.phdrs.swap(0, 2);
    assert_eq!(refused(&elf), Error::Unsorted);
}

#[test]
fn refuses_loads_that_share_a_page() {
    let mut elf = base();
    elf.phdrs[1] = load(PF_R, 0x2800, 0x40_1800, 0x20, 0x20);
    assert_eq!(refused(&elf), Error::Overlap);
    let mut elf = base();
    elf.phdrs[1] = load(PF_R, 0x1010, 0x40_1010, 0x10, 0x10);
    assert_eq!(refused(&elf), Error::Overlap);
    let mut elf = base();
    elf.phdrs[1].vaddr = 0x40_0000;
    elf.phdrs[1].offset = 0x2000;
    assert_eq!(refused(&elf), Error::Overlap);
}

#[test]
fn adjacent_pages_do_not_overlap() {
    let mut elf = base();
    elf.phdrs[1] = load(PF_R, 0x2000, 0x40_2000, 0x20, 0x20);
    elf.phdrs[2] = load(PF_R | PF_W, 0x3000, 0x40_3000, 0x10, 0x10);
    assert!(elf.parse().is_ok());
}

#[test]
fn refuses_total_memsz_above_the_ceiling() {
    let mut small = limits();
    small.max_loads_bytes = 0x1010 + 0x20 + 0x2000 - 1;
    assert_eq!(parse(&base().build(), &small), Err(Error::TooLarge));
    small.max_loads_bytes += 1;
    assert!(parse(&base().build(), &small).is_ok());
    small.max_loads_bytes = 0;
    assert_eq!(parse(&base().build(), &small), Err(Error::TooLarge));
}

#[test]
fn memsz_sum_cannot_overflow_inside_the_user_window() {
    let mut huge = limits();
    huge.max_loads_bytes = u64::MAX;
    huge.user_min = 0;
    huge.user_top = 1 << 63;
    let mut elf = base();
    elf.phdrs = (0..16)
        .map(|i| load(PF_R, 0x1000, i * (1 << 59), 0, 1 << 59))
        .collect();
    elf.entry = 0;
    assert_eq!(parse(&elf.build(), &huge), Err(Error::EntryNotExecutable));
}

#[test]
fn refuses_entry_outside_executable_file_content() {
    for entry in [
        0,
        0x40_2000,
        0x40_3000,
        0x40_3fff,
        0x40_5000,
        u64::MAX,
        USER_TOP,
    ] {
        let mut elf = base();
        elf.entry = entry;
        assert_eq!(refused(&elf), Error::EntryNotExecutable, "{entry:#x}");
    }
}

#[test]
fn entry_at_the_edges_of_the_executable_file_content() {
    let mut elf = base();
    elf.entry = 0x40_0000;
    assert!(elf.parse().is_ok());
    elf.entry = 0x40_100f;
    assert!(elf.parse().is_ok());
    elf.entry = 0x40_1010;
    assert_eq!(refused(&elf), Error::EntryNotExecutable);
}

#[test]
fn entry_in_bss_of_an_executable_load_is_refused() {
    let mut elf = base();
    elf.phdrs[0].memsz = 0x2000;
    elf.phdrs[1].vaddr = 0x40_4000;
    elf.phdrs[2].vaddr = 0x40_5000;
    elf.phdrs[1].offset = 0x2000;
    elf.phdrs[2].offset = 0x3000;
    elf.entry = 0x40_1800;
    assert_eq!(refused(&elf), Error::EntryNotExecutable);
}

#[test]
fn refuses_image_without_loads() {
    let mut elf = with_extra(4);
    elf.phdrs.drain(..3);
    assert_eq!(refused(&elf), Error::EntryNotExecutable);
}

#[test]
fn refuses_bad_limits() {
    for page in [0, 3, 4095, 6144] {
        let mut bad = limits();
        bad.page_size = page;
        assert_eq!(parse(&base().build(), &bad), Err(Error::BadLimits));
    }
    let mut bad = limits();
    bad.user_min = bad.user_top + 1;
    assert_eq!(parse(&base().build(), &bad), Err(Error::BadLimits));
}

#[test]
fn small_pages_work() {
    let mut small = limits();
    small.page_size = 1;
    assert!(parse(&base().build(), &small).is_ok());
}

#[test]
fn every_truncation_of_a_valid_file_is_refused() {
    let bytes = base().build();
    for len in 0..0x3010 {
        assert!(parse(&bytes[..len], &limits()).is_err(), "len {len}");
    }
    assert!(parse(&bytes[..0x3010], &limits()).is_ok());
    assert!(parse(&bytes, &limits()).is_ok());
}

#[test]
fn truncation_errors_are_the_expected_ones() {
    let bytes = base().build();
    assert_eq!(parse(&bytes[..0], &limits()), Err(Error::Truncated));
    assert_eq!(parse(&bytes[..63], &limits()), Err(Error::Truncated));
    assert_eq!(
        parse(&bytes[..64], &limits()),
        Err(Error::PhdrTableOutOfFile)
    );
    assert_eq!(
        parse(&bytes[..231], &limits()),
        Err(Error::PhdrTableOutOfFile)
    );
    assert_eq!(
        parse(&bytes[..232], &limits()),
        Err(Error::SegmentOutOfFile)
    );
}

#[test]
fn corrupting_any_header_byte_never_panics() {
    let base_bytes = base().build();
    for index in 0..232 {
        for value in [0u8, 1, 0x7f, 0x80, 0xff] {
            let mut bytes = base_bytes.clone();
            bytes[index] = value;
            let _ = parse(&bytes, &limits());
        }
    }
}

#[test]
fn every_field_at_the_extremes_never_panics() {
    let extremes = [
        0u64,
        1,
        0xfff,
        0x1000,
        u64::MAX,
        u64::MAX - 1,
        1 << 63,
        (1 << 47) + 1,
    ];
    for field in 0..7 {
        for &value in &extremes {
            for index in 0..3 {
                let elf = with_load(index, |p| match field {
                    0 => p.flags = u32::try_from(value & 0xffff_ffff).unwrap(),
                    1 => p.offset = value,
                    2 => p.vaddr = value,
                    3 => p.filesz = value,
                    4 => p.memsz = value,
                    5 => p.align = value,
                    _ => p.kind = u32::try_from(value & 0xffff_ffff).unwrap(),
                });
                let _ = elf.parse();
            }
        }
    }
    for phoff in extremes {
        let mut elf = base();
        elf.phoff = phoff;
        let _ = elf.parse();
        let mut elf = base();
        elf.entry = phoff;
        let _ = elf.parse();
    }
}

/// `rustc 1.100.0`, no desenvolvimento, com o linker script `user.ld`: só os 232
/// primeiros bytes (cabeçalho e 3 program headers). A linha de comando está no
/// SDD 03; o programa não entra no build.
const REAL_X86_64: &str = concat!(
    "7f454c4602010100000000000000000002003e00010000000000400000000000",
    "4000000000000000883100000000000000000000400038000300400008000600",
    "0100000005000000001000000000000000004000000000000000400000000000",
    "1600000000000000160000000000000000100000000000000100000004000000",
    "0020000000000000001040000000000000104000000000001000000000000000",
    "1000000000000000001000000000000001000000060000000030000000000000",
    "0020400000000000002040000000000004000000000000000401000000000000",
    "0010000000000000",
);
const REAL_X86_64_LEN: usize = 13192;

const REAL_AARCH64: &str = concat!(
    "7f454c460201010000000000000000000200b700010000000000400000000000",
    "4000000000000000d03100000000000000000000400038000300400008000600",
    "0100000005000000001000000000000000004000000000000000400000000000",
    "2800000000000000280000000000000000100000000000000100000004000000",
    "0020000000000000001040000000000000104000000000001000000000000000",
    "1000000000000000001000000000000001000000060000000030000000000000",
    "0020400000000000002040000000000004000000000000000401000000000000",
    "0010000000000000",
);
const REAL_AARCH64_LEN: usize = 13264;

fn real(hex: &str, len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; len];
    for index in 0..hex.len() / 2 {
        bytes[index] = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap();
    }
    bytes
}

fn check_real(bytes: &[u8], machine: u16) {
    let mut limits = limits();
    limits.machine = machine;
    let image = parse(bytes, &limits).unwrap();
    assert_eq!(image.entry, 0x40_0000);
    assert_eq!(image.phnum, 3);
    assert_eq!(image.phdr, None);
    let loads = image.loads();
    assert_eq!(loads.len(), 3);
    assert_eq!(
        (loads[0].vaddr, loads[0].offset, loads[0].flags),
        (0x40_0000, 0x1000, PF_R | PF_X)
    );
    assert_eq!(
        (loads[1].vaddr, loads[1].offset, loads[1].flags),
        (0x40_1000, 0x2000, PF_R)
    );
    assert_eq!(
        (loads[2].vaddr, loads[2].offset, loads[2].flags),
        (0x40_2000, 0x3000, PF_R | PF_W)
    );
    assert_eq!((loads[2].filesz, loads[2].memsz), (4, 260));
    for segment in loads {
        assert_eq!(
            segment.file_bytes(bytes).map(<[u8]>::len),
            Some(usize::try_from(segment.filesz).unwrap())
        );
    }
    for len in 0..0x3004 {
        assert!(parse(&bytes[..len], &limits).is_err(), "len {len}");
    }
}

#[test]
fn real_x86_64_elf_is_accepted() {
    let bytes = real(REAL_X86_64, REAL_X86_64_LEN);
    assert_eq!(&bytes[18..20], &EM_X86_64.to_le_bytes());
    check_real(&bytes, EM_X86_64);
}

#[test]
fn real_aarch64_elf_is_accepted() {
    let bytes = real(REAL_AARCH64, REAL_AARCH64_LEN);
    assert_eq!(&bytes[18..20], &EM_AARCH64.to_le_bytes());
    check_real(&bytes, EM_AARCH64);
}

#[test]
fn real_elf_of_the_other_isa_is_refused() {
    let x86 = real(REAL_X86_64, REAL_X86_64_LEN);
    let mut arm = limits();
    arm.machine = EM_AARCH64;
    assert_eq!(parse(&x86, &arm), Err(Error::BadMachine));
    let aarch64 = real(REAL_AARCH64, REAL_AARCH64_LEN);
    assert_eq!(parse(&aarch64, &limits()), Err(Error::BadMachine));
}

#[test]
fn real_elf_with_the_gnu_osabi_lld_writes_for_retained_statics_is_refused() {
    let mut bytes = real(REAL_X86_64, REAL_X86_64_LEN);
    bytes[7] = 3;
    assert_eq!(parse(&bytes, &limits()), Err(Error::BadOsAbi));
}

#[test]
fn real_elf_as_position_independent_is_refused() {
    let mut bytes = real(REAL_X86_64, REAL_X86_64_LEN);
    bytes[16] = 3;
    assert_eq!(parse(&bytes, &limits()), Err(Error::BadType));
}

#[test]
fn flags_keep_only_the_rwx_bits() {
    let elf = with_load(1, |p| p.flags = PF_R | 0x0ff0_0000 | 0xf000_0000);
    let segment = elf.parse().unwrap().loads()[1];
    assert_eq!(segment.flags, PF_R);
    assert!(segment.readable() && !segment.writable() && !segment.executable());
    let elf = with_load(2, |p| p.flags = PF_R | PF_W | 0x0010_0000);
    let segment = elf.parse().unwrap().loads()[2];
    assert_eq!(segment.flags, PF_R | PF_W);
    assert!(segment.readable() && segment.writable());
}

#[test]
fn page_range_and_offset_round_outwards() {
    let elf = with_load(2, |p| {
        p.vaddr = 0x40_3010;
        p.offset = 0x3010;
        p.filesz = 0;
        p.memsz = 0x2000;
    });
    let segment = elf.parse().unwrap().loads()[2];
    assert_eq!(segment.page_range(PAGE), (0x40_3000, 0x40_6000));
    assert_eq!(segment.page_offset(PAGE), 0x10);
    let aligned = base().parse().unwrap().loads()[2];
    assert_eq!(aligned.page_range(PAGE), (0x40_3000, 0x40_5000));
    assert_eq!(aligned.page_offset(PAGE), 0);
    let tiny = Segment {
        vaddr: 0x40_3fff,
        memsz: 1,
        ..Segment::EMPTY
    };
    assert_eq!(tiny.page_range(PAGE), (0x40_3000, 0x40_4000));
    assert_eq!(tiny.page_range(1), (0x40_3fff, 0x40_4000));
    let huge = Segment {
        vaddr: u64::MAX,
        memsz: u64::MAX,
        ..Segment::EMPTY
    };
    assert_eq!(huge.page_range(0), (0, 0));
    assert_eq!(huge.page_range(PAGE), (!0xfff, !0xfff));
}

#[test]
fn file_bytes_start_at_vaddr_not_at_the_page() {
    let mut elf = base();
    elf.phdrs[2] = load(PF_R | PF_W, 0x3010, 0x40_3010, 0x8, 0x1000);
    let mut bytes = elf.build();
    bytes[0x300f] = 0x11;
    bytes[0x3010] = 0x5a;
    let segment = parse(&bytes, &limits()).unwrap().loads()[2];
    assert_eq!(segment.page_offset(PAGE), 0x10);
    let data = segment.file_bytes(&bytes).unwrap();
    assert_eq!(data.len(), 8);
    assert_eq!(data[0], 0x5a);
}

#[test]
fn limits_are_checked_after_rounding_to_the_page() {
    // O segmento termina em 0x40_3010, antes de `user_top`, mas a página dele vai a 0x40_4000.
    let mut elf = base();
    elf.phdrs[2] = load(PF_R | PF_W, 0x3000, 0x40_3000, 0x10, 0x10);
    let mut tight = limits();
    tight.user_top = 0x40_3800;
    assert_eq!(parse(&elf.build(), &tight), Err(Error::OutOfUserRange));
    tight.user_top = 0x40_4000;
    assert!(parse(&elf.build(), &tight).is_ok());
    // O segmento começa em 0x40_0900, depois de `user_min`, mas a página dele começa em 0x40_0000.
    let mut elf = base();
    elf.phdrs[0] = load(PF_R | PF_X, 0x900, 0x40_0900, 0x1010, 0x1010);
    elf.entry = 0x40_1000;
    let mut tight = limits();
    tight.user_min = 0x40_0800;
    assert_eq!(parse(&elf.build(), &tight), Err(Error::OutOfUserRange));
    tight.user_min = 0x40_0000;
    assert!(parse(&elf.build(), &tight).is_ok());
}

#[test]
fn phdr_address_at_the_edges_of_the_load() {
    // A tabela vai de 64 a 232 no arquivo.
    let at = |offset: u64, filesz: u64| {
        let mut elf = base();
        elf.phdrs[0] = load(
            PF_R | PF_X,
            offset,
            0x40_0000 + offset,
            filesz,
            filesz.max(0x1010),
        );
        elf.entry = 0x40_0000 + offset;
        elf.parse().unwrap().phdr
    };
    assert_eq!(at(64, 0x1010), Some(0x40_0040));
    assert_eq!(at(65, 0x1010), None);
    assert_eq!(at(0, 232), Some(0x40_0040));
    assert_eq!(at(0, 231), None);
    assert_eq!(at(0, 233), Some(0x40_0040));
}
