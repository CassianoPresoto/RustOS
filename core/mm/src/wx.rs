//! Confere as tabelas de página do kernel: nenhuma página é gravável e executável ao
//! mesmo tempo (guidelines 09), cada seção da imagem tem a permissão que o linker
//! script deu, e a metade baixa e o alias da imagem no mapa direto não existem.

use core::fmt;

use rustos_hal::{Mapping, PAGE_SIZE, PageFlags};

/// Menor endereço que o kernel usa nas duas ISAs (o do `TTBR1_EL1` com 48 bits; no
/// x86_64 a metade alta começa mais acima). Abaixo dele fica a metade de usuário.
pub const KERNEL_HALF: u64 = 0xffff_0000_0000_0000;
/// Quantas seções o conferidor aceita: `.requests`, `.text`, `.rodata`, `.data` com
/// `.bss` e a faixa `.data.rel.ro`, que mora dentro de `.rodata`.
pub const MAX_SECTIONS: usize = 5;

/// Uma seção da imagem: faixa virtual e a permissão que ela deve ter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Section {
    pub name: &'static str,
    pub start: u64,
    pub end: u64,
    pub flags: PageFlags,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Violation {
    /// Página gravável e executável.
    WriteExecute(u64),
    /// Página executável que não cabe inteira em seção executável (`.text`).
    ExecuteOutsideText(u64),
    /// Seção com permissão diferente da esperada.
    SectionFlags(&'static str, u64),
    /// Parte da seção sem mapeamento.
    SectionUnmapped(&'static str),
    /// Mapeamento abaixo da metade alta.
    LowerHalf(u64),
    /// Outro mapeamento aponta para a memória física da imagem.
    ImageAlias(u64),
    /// A faixa `.data.rel.ro` não contém a tabela relocada que o kernel plantou nela.
    RelroMissing,
}

/// Os endereços saem em hexadecimal, o que o filtro do log reconhece (SDD 09).
impl fmt::Debug for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WriteExecute(virt) => write!(f, "WriteExecute({virt:#x})"),
            Self::ExecuteOutsideText(virt) => write!(f, "ExecuteOutsideText({virt:#x})"),
            Self::SectionFlags(name, virt) => write!(f, "SectionFlags({name:?}, {virt:#x})"),
            Self::SectionUnmapped(name) => write!(f, "SectionUnmapped({name:?})"),
            Self::LowerHalf(virt) => write!(f, "LowerHalf({virt:#x})"),
            Self::ImageAlias(virt) => write!(f, "ImageAlias({virt:#x})"),
            Self::RelroMissing => f.write_str("RelroMissing"),
        }
    }
}

/// Recebe as folhas, em qualquer ordem, e diz no fim se alguma regra falhou.
#[derive(Debug)]
pub struct WxChecker<'a> {
    sections: &'a [Section],
    image_phys: (u64, u64),
    covered: [u64; MAX_SECTIONS],
    pages: u64,
    violation: Option<Violation>,
}

impl<'a> WxChecker<'a> {
    /// `image_phys` é a faixa física `[início, fim)` da imagem do kernel.
    ///
    /// # Panics
    /// Com mais de `MAX_SECTIONS` seções.
    pub fn new(sections: &'a [Section], image_phys: (u64, u64)) -> Self {
        assert!(
            sections.len() <= MAX_SECTIONS,
            "seções demais para o conferidor"
        );
        Self {
            sections,
            image_phys,
            covered: [0; MAX_SECTIONS],
            pages: 0,
            violation: None,
        }
    }

    pub fn visit(&mut self, mapping: Mapping) {
        let virt = mapping.virt.as_u64();
        self.pages += mapping.size / PAGE_SIZE;
        if mapping.flags.violates_wx() {
            self.fail(Violation::WriteExecute(virt));
        }
        if virt < KERNEL_HALF {
            self.fail(Violation::LowerHalf(virt));
        }
        let mut in_section = false;
        let mut in_text = false;
        for (index, section) in self.sections.iter().enumerate() {
            let overlap = overlap(virt, virt + mapping.size, section.start, section.end);
            if overlap == 0 {
                continue;
            }
            in_section = true;
            in_text |= section.flags.executable
                && virt >= section.start
                && virt + mapping.size <= section.end;
            self.covered[index] += overlap;
            let same = mapping.flags.writable == section.flags.writable
                && mapping.flags.executable == section.flags.executable;
            if !same {
                self.fail(Violation::SectionFlags(section.name, virt));
            }
        }
        if mapping.flags.executable && !in_text {
            self.fail(Violation::ExecuteOutsideText(virt));
        }
        let phys = mapping.phys.as_u64();
        let alias = overlap(
            phys,
            phys + mapping.size,
            self.image_phys.0,
            self.image_phys.1,
        ) > 0;
        if alias && !in_section {
            self.fail(Violation::ImageAlias(virt));
        }
    }

    fn fail(&mut self, violation: Violation) {
        self.violation.get_or_insert(violation);
    }

    /// As páginas de 4 KiB conferidas, ou a primeira regra que falhou.
    pub fn finish(mut self) -> Result<u64, Violation> {
        for (index, section) in self.sections.iter().enumerate() {
            if self.covered[index] != section.end - section.start {
                self.fail(Violation::SectionUnmapped(section.name));
            }
        }
        self.violation.map_or(Ok(self.pages), Err)
    }
}

const fn overlap(a_start: u64, a_end: u64, b_start: u64, b_end: u64) -> u64 {
    let start = if a_start > b_start { a_start } else { b_start };
    let end = if a_end < b_end { a_end } else { b_end };
    end.saturating_sub(start)
}

#[cfg(test)]
mod tests {
    use rustos_hal::{CacheMode, PhysAddr, VirtAddr};

    use super::*;

    const IMAGE_VIRT: u64 = 0xffff_ffff_8000_0000;
    const IMAGE_PHYS: u64 = 0x20_0000;
    const HHDM: u64 = 0xffff_8000_0000_0000;

    fn sections() -> [Section; 3] {
        [
            Section {
                name: ".text",
                start: IMAGE_VIRT,
                end: IMAGE_VIRT + 2 * PAGE_SIZE,
                flags: PageFlags::CODE,
            },
            Section {
                name: ".rodata",
                start: IMAGE_VIRT + 2 * PAGE_SIZE,
                end: IMAGE_VIRT + 3 * PAGE_SIZE,
                flags: PageFlags::RODATA,
            },
            Section {
                name: ".data",
                start: IMAGE_VIRT + 3 * PAGE_SIZE,
                end: IMAGE_VIRT + 4 * PAGE_SIZE,
                flags: PageFlags::DATA,
            },
        ]
    }

    fn page(virt: u64, phys: u64, flags: PageFlags) -> Mapping {
        Mapping {
            virt: VirtAddr::new(virt),
            phys: PhysAddr::new(phys),
            size: PAGE_SIZE,
            flags,
        }
    }

    fn page_at(index: u64, flags: PageFlags) -> Mapping {
        page(
            IMAGE_VIRT + index * PAGE_SIZE,
            IMAGE_PHYS + index * PAGE_SIZE,
            flags,
        )
    }

    /// Imagem correta mais um bloco de RAM no mapa direto, longe da imagem.
    fn good() -> Vec<Mapping> {
        let flags = [
            PageFlags::CODE,
            PageFlags::CODE,
            PageFlags::RODATA,
            PageFlags::DATA,
        ];
        let mut maps: Vec<Mapping> = (0..)
            .zip(flags)
            .map(|(n, flags)| {
                page(
                    IMAGE_VIRT + n * PAGE_SIZE,
                    IMAGE_PHYS + n * PAGE_SIZE,
                    flags,
                )
            })
            .collect();
        maps.push(Mapping {
            virt: VirtAddr::new(HHDM + 0x1000_0000),
            phys: PhysAddr::new(0x1000_0000),
            size: 0x20_0000,
            flags: PageFlags::DATA,
        });
        maps
    }

    fn run(maps: &[Mapping]) -> Result<u64, Violation> {
        let sections = sections();
        let mut checker = WxChecker::new(&sections, (IMAGE_PHYS, IMAGE_PHYS + 4 * PAGE_SIZE));
        for &mapping in maps {
            checker.visit(mapping);
        }
        checker.finish()
    }

    #[test]
    fn violations_print_their_addresses_in_hex() {
        use std::format;
        assert_eq!(
            format!("{:?}", Violation::WriteExecute(0xffff_8000_0000_1000)),
            "WriteExecute(0xffff800000001000)"
        );
        assert_eq!(
            format!("{:?}", Violation::SectionFlags(".text", 0x1000)),
            "SectionFlags(\".text\", 0x1000)"
        );
    }

    #[test]
    fn a_correct_space_passes_and_counts_4k_pages() {
        assert_eq!(run(&good()), Ok(4 + 512));
    }

    #[test]
    fn writable_and_executable_page_is_refused_anywhere() {
        let mut maps = good();
        let both = PageFlags {
            writable: true,
            executable: true,
            user: false,
            cache: CacheMode::WriteBack,
        };
        maps.push(page(HHDM + 0x2000_0000, 0x2000_0000, both));
        assert_eq!(run(&maps), Err(Violation::WriteExecute(HHDM + 0x2000_0000)));
    }

    #[test]
    fn an_executable_page_outside_text_is_refused() {
        let mut maps = good();
        maps.push(page(HHDM + 0x2000_0000, 0x2000_0000, PageFlags::CODE));
        assert_eq!(
            run(&maps),
            Err(Violation::ExecuteOutsideText(HHDM + 0x2000_0000))
        );
    }

    #[test]
    fn an_executable_large_page_that_spills_out_of_text_is_refused() {
        let big = Mapping {
            virt: VirtAddr::new(IMAGE_VIRT),
            phys: PhysAddr::new(IMAGE_PHYS),
            size: 0x20_0000,
            flags: PageFlags::CODE,
        };
        let sections = &sections()[..1];
        let mut checker = WxChecker::new(sections, (IMAGE_PHYS, IMAGE_PHYS + 2 * PAGE_SIZE));
        checker.visit(big);
        assert_eq!(
            checker.finish(),
            Err(Violation::ExecuteOutsideText(IMAGE_VIRT))
        );
    }

    #[test]
    fn a_relro_range_that_is_writable_is_refused() {
        let at = |page: u64| IMAGE_VIRT + page * PAGE_SIZE;
        let section = |name, page, flags| Section {
            name,
            start: at(page),
            end: at(page + 1),
            flags,
        };
        let sections = [
            section(".text", 0, PageFlags::CODE),
            section(".data.rel.ro", 1, PageFlags::RODATA),
            section(".data", 2, PageFlags::DATA),
        ];
        let verdict = |relro: PageFlags| {
            let mut checker = WxChecker::new(&sections, (IMAGE_PHYS, IMAGE_PHYS + 3 * PAGE_SIZE));
            let flags = [PageFlags::CODE, relro, PageFlags::DATA];
            for (page, flags) in (0..).zip(flags) {
                checker.visit(page_at(page, flags));
            }
            checker.finish()
        };
        assert_eq!(verdict(PageFlags::RODATA), Ok(3));
        assert_eq!(
            verdict(PageFlags::DATA),
            Err(Violation::SectionFlags(".data.rel.ro", at(1)))
        );
    }

    #[test]
    fn writable_text_is_refused() {
        let mut maps = good();
        maps[0].flags = PageFlags::DATA;
        let sections_error = Violation::SectionFlags(".text", IMAGE_VIRT);
        assert_eq!(run(&maps), Err(sections_error));
    }

    #[test]
    fn executable_rodata_is_refused() {
        let mut maps = good();
        maps[2].flags = PageFlags::CODE;
        assert_eq!(
            run(&maps),
            Err(Violation::SectionFlags(
                ".rodata",
                IMAGE_VIRT + 2 * PAGE_SIZE
            ))
        );
    }

    #[test]
    fn a_missing_page_leaves_the_section_unmapped() {
        let mut maps = good();
        maps.remove(1);
        assert_eq!(run(&maps), Err(Violation::SectionUnmapped(".text")));
    }

    #[test]
    fn a_large_mapping_over_the_image_covers_it_and_fails_the_flags() {
        let big = Mapping {
            virt: VirtAddr::new(IMAGE_VIRT),
            phys: PhysAddr::new(IMAGE_PHYS),
            size: 0x20_0000,
            flags: PageFlags::DATA,
        };
        assert_eq!(
            run(&[big]),
            Err(Violation::SectionFlags(".text", IMAGE_VIRT))
        );
    }

    #[test]
    fn a_direct_map_alias_of_the_image_is_refused() {
        let mut maps = good();
        maps.push(page(HHDM + IMAGE_PHYS, IMAGE_PHYS, PageFlags::DATA));
        assert_eq!(run(&maps), Err(Violation::ImageAlias(HHDM + IMAGE_PHYS)));
    }

    #[test]
    fn a_lower_half_mapping_is_refused() {
        let mut maps = good();
        maps.push(page(0x1000, 0x5000_0000, PageFlags::DATA));
        assert_eq!(run(&maps), Err(Violation::LowerHalf(0x1000)));
    }

    #[test]
    fn the_first_violation_is_the_one_reported() {
        let mut maps = good();
        maps.push(page(0x1000, 0x5000_0000, PageFlags::DATA));
        maps.push(page(HHDM + IMAGE_PHYS, IMAGE_PHYS, PageFlags::DATA));
        assert_eq!(run(&maps), Err(Violation::LowerHalf(0x1000)));
    }
}
