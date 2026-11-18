//! Espaço de endereçamento do kernel: tabelas novas, uma permissão por seção da
//! imagem e o mapa direto refeito. A troca para elas é `Vm::activate`.

use rustos_hal::{
    AddressSpace, BootInfo, FrameSource, MapError, PAGE_SIZE, PageFlags, PageMapper, PhysAddr,
    RegionKind, VirtAddr,
};
use rustos_mm::ranges::{coalesce, runs};
use rustos_mm::{Section, Violation, WxChecker};

use crate::frames::Frames;
use crate::platform::{self, PlatformSpace};

unsafe extern "C" {
    static __image_start: u8;
    static __requests_end: u8;
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __data_end: u8;
    static __relro_start: u8;
    static __relro_end: u8;
}

/// A faixa virtual da imagem, `[__image_start, __data_end)`.
pub fn image_range() -> (u64, u64) {
    (
        symbol(&raw const __image_start),
        symbol(&raw const __data_end),
    )
}

static RELRO_MARK: u8 = 0;
/// Tabela de ponteiros relocados: o linker tem de pô-la em `.data.rel.ro`.
static RELRO_CANARY: [&u8; 1] = [&RELRO_MARK];

#[derive(Debug)]
pub enum VmError {
    Map(#[allow(dead_code, reason = "lido pelo Debug da mensagem de pânico")] MapError),
    NoKernelImage,
    /// O endereço virtual que o bootloader deu não é o do linker script.
    ImageMismatch,
}

impl From<MapError> for VmError {
    fn from(error: MapError) -> Self {
        Self::Map(error)
    }
}

/// O que `Vm::build` montou, para o relatório do boot.
#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "lido só pelo modo de teste")
)]
#[derive(Clone, Copy, Debug, Default)]
pub struct VmStats {
    pub table_frames: u64,
    pub large_pages: u64,
    pub small_pages: u64,
}

/// Conta os frames que o mapeador gasta em tabelas.
struct Counting<'a> {
    frames: &'a mut Frames,
    taken: u64,
}

impl FrameSource for Counting<'_> {
    fn allocate_frame(&mut self) -> Option<PhysAddr> {
        let frame = self.frames.allocate_frame();
        self.taken += u64::from(frame.is_some());
        frame
    }
}

#[derive(Debug)]
pub struct Vm {
    space: PlatformSpace,
    sections: [Section; 4],
    image_phys: (u64, u64),
    /// Faixa virtual do mapa direto, `[início, fim)`.
    direct: (u64, u64),
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "lido só pelo modo de teste")
    )]
    stats: VmStats,
}

fn symbol(address: *const u8) -> u64 {
    address.addr() as u64
}

fn image_sections() -> [Section; 4] {
    let range = |name, start: *const u8, end: *const u8, flags| Section {
        name,
        start: symbol(start),
        end: symbol(end),
        flags,
    };
    [
        range(
            ".requests",
            &raw const __image_start,
            &raw const __requests_end,
            PageFlags::RODATA,
        ),
        range(
            ".text",
            &raw const __text_start,
            &raw const __text_end,
            PageFlags::CODE,
        ),
        range(
            ".rodata",
            &raw const __rodata_start,
            &raw const __rodata_end,
            PageFlags::RODATA,
        ),
        range(
            ".data",
            &raw const __data_start,
            &raw const __data_end,
            PageFlags::DATA,
        ),
    ]
}

/// Mapeia `[phys, phys + len)` em `virt`, de 2 MiB em 2 MiB onde a política deixa.
fn map_range(
    space: &mut PlatformSpace,
    frames: &mut Counting<'_>,
    stats: &mut VmStats,
    virt: u64,
    phys: u64,
    len: u64,
    flags: PageFlags,
) -> Result<(), MapError> {
    for run in runs(virt, phys, len) {
        let (virt, phys) = (VirtAddr::new(run.virt), PhysAddr::new(run.phys));
        if run.large {
            space.map_large_page(virt, phys, flags, frames)?;
            stats.large_pages += 1;
        } else {
            space.map_page(virt, phys, flags, frames)?;
            stats.small_pages += 1;
        }
    }
    Ok(())
}

impl Vm {
    /// Monta o espaço sem ativá-lo: imagem por seção, RAM e framebuffer no mapa direto.
    /// # Safety
    /// Todo frame do alocador e toda região utilizável ou recuperável de `boot` é
    /// acessível em `físico + offset` agora, pelo mapa direto do bootloader.
    pub unsafe fn build(boot: &impl BootInfo, frames: &mut Frames) -> Result<Self, VmError> {
        let image = boot.kernel_image().ok_or(VmError::NoKernelImage)?;
        let sections = image_sections();
        if sections[0].start != image.virtual_base.as_u64() {
            return Err(VmError::ImageMismatch);
        }
        let offset = boot.direct_map_offset();
        let mut frames = Counting { frames, taken: 0 };
        // SAFETY: o contrato desta função dá o acesso aos frames pelo mapa direto.
        let mut space = unsafe { platform::new_space(&mut frames, offset) }?;
        let mut stats = VmStats::default();

        for section in &sections {
            let phys = image
                .physical_of(VirtAddr::new(section.start))
                .ok_or(VmError::ImageMismatch)?;
            let len = section.end - section.start;
            let (virt, phys) = (section.start, phys.as_u64());
            map_range(
                &mut space,
                &mut frames,
                &mut stats,
                virt,
                phys,
                len,
                section.flags,
            )?;
        }
        let image_end = sections[3].end;
        let image_phys = image
            .physical_of(VirtAddr::new(sections[0].start))
            .zip(image.physical_of(VirtAddr::new(image_end)))
            .map(|(start, end)| (start.as_u64(), end.as_u64()))
            .ok_or(VmError::ImageMismatch)?;

        let direct = map_direct(boot, &mut space, &mut frames, &mut stats)?;
        stats.table_frames = frames.taken;
        Ok(Self {
            space,
            sections,
            image_phys,
            direct,
            stats,
        })
    }

    /// Passa a traduzir pelas tabelas novas.
    pub fn activate(&self) {
        self.space.activate();
    }

    pub fn space_mut(&mut self) -> &mut PlatformSpace {
        &mut self.space
    }

    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "lido só pelo modo de teste")
    )]
    pub fn stats(&self) -> VmStats {
        self.stats
    }

    /// As faixas virtuais que o bootloader sorteia: mapa direto e imagem. As janelas
    /// do kernel (dispositivos, pilhas) ficam fora delas.
    pub fn occupied(&self) -> [(u64, u64); 2] {
        [self.direct, (self.sections[0].start, self.sections[3].end)]
    }

    /// Percorre as tabelas: nenhuma página gravável e executável, cada seção com a
    /// permissão do linker script. Devolve as páginas de 4 KiB conferidas.
    pub fn check_wx(&self) -> Result<u64, Violation> {
        let relro = Section {
            name: ".data.rel.ro",
            start: symbol(&raw const __relro_start),
            end: symbol(&raw const __relro_end),
            flags: PageFlags::RODATA,
        };
        let [requests, text, rodata, data] = self.sections;
        let sections = [requests, text, rodata, data, relro];
        let mut checker = WxChecker::new(&sections, self.image_phys);
        self.space.walk(&mut |mapping| checker.visit(mapping));
        let pages = checker.finish()?;
        let canary = core::hint::black_box(&raw const RELRO_CANARY).addr() as u64;
        if canary < sections[4].start || canary >= sections[4].end {
            return Err(Violation::RelroMissing);
        }
        Ok(pages)
    }
}

/// RAM do kernel e do bootloader no mapa direto (leitura e escrita, sem execução)
/// e o framebuffer com o cache de vídeo. Dispositivo e a própria imagem ficam de fora.
/// Devolve a faixa virtual que o mapa ocupa.
fn map_direct(
    boot: &impl BootInfo,
    space: &mut PlatformSpace,
    frames: &mut Counting<'_>,
    stats: &mut VmStats,
) -> Result<(u64, u64), MapError> {
    let offset = boot.direct_map_offset();
    let mut span = (u64::MAX, 0);
    let mut map = |start: u64, end: u64, flags| {
        span = (span.0.min(start), span.1.max(end));
        let len = end - start;
        map_range(space, frames, stats, offset + start, start, len, flags)
    };
    let ram = boot
        .memory_regions()
        .filter(|region| {
            matches!(
                region.kind,
                RegionKind::Usable | RegionKind::BootloaderReclaimable
            )
        })
        .map(|region| (region.start.as_u64(), region.end()));
    for (start, end) in coalesce(ram) {
        map(start, end, PageFlags::DATA)?;
    }
    if let Some(info) = boot.framebuffer() {
        let start = info
            .address
            .as_u64()
            .checked_sub(offset)
            .ok_or(MapError::OutOfAddressSpace)?;
        let first = start & !(PAGE_SIZE - 1);
        let last = (start + info.len_bytes()).next_multiple_of(PAGE_SIZE);
        map(first, last, PageFlags::FRAMEBUFFER)?;
    }
    Ok((offset.saturating_add(span.0), offset.saturating_add(span.1)))
}

/// Escreve em `.text` o byte que já está lá: com a proteção ligada é um page fault de
/// proteção e não volta; sem ela, a escrita não muda nada e a função volta.
pub fn probe_write_text() {
    let target = probe_write_text as *const () as *mut u8;
    // SAFETY: `target` é a primeira instrução desta função, em `.text`, só lida aqui.
    let byte = unsafe { target.read_volatile() };
    // SAFETY: regrava o byte lido; só provoca o page fault que `.text` deve dar.
    unsafe { target.write_volatile(byte) };
}
