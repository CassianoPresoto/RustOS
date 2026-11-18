//! Adaptador do protocolo de boot Limine (revisão base 6): traduz as respostas
//! do bootloader para `hal::BootInfo`. Protocolo escrito aqui, sem crate externo.

#![cfg_attr(not(test), no_std)]

mod protocol;
mod requests;

use core::ffi::{CStr, c_char};
use core::sync::atomic::Ordering;
use core::{ptr, slice};

use rustos_hal::{
    BootInfo, BootModule, BootTimeline, BootloaderId, Channel, CpuDescriptor, FramebufferInfo,
    KernelImage, MemoryRegion, PAGE_SIZE, PhysAddr, RegionKind, VirtAddr,
};

pub use protocol::{BASE_REVISION, MpInfo};
use protocol::{File, Framebuffer, MemmapEntry};

/// Maior `string` de módulo que o kernel guarda; uma maior, sem NUL ou que não é UTF-8 vira vazia.
const MODULE_STRING_MAX: usize = 256;
/// Maior módulo aceito: a janela de módulos do kernel (SDD 13) tem 1 GiB, e um maior não cabe.
const MODULE_MAX_BYTES: u64 = 1 << 30;

/// Entrada de um núcleo secundário: recebe o `MpInfo` do próprio núcleo (`RDI` ou `X0`).
pub type CpuEntry = unsafe extern "C" fn(info: *const MpInfo) -> !;

/// Solta o núcleo secundário `token`: grava o argumento e o `goto_address`, com release.
/// # Safety
/// `token` é de `CpuDescriptor` de um núcleo que não é o de boot, solto uma vez, com a
/// memória do bootloader intacta.
pub unsafe fn start_cpu(token: usize, argument: u64, entry: CpuEntry) {
    let info = ptr::with_exposed_provenance_mut::<MpInfo>(token);
    // SAFETY: `token` aponta para o `MpInfo` do núcleo, pelo contrato da função.
    let argument_field = unsafe { &raw mut (*info).extra_argument };
    // SAFETY: o campo está dentro do `MpInfo`, e o núcleo só o lê depois do `goto_address`.
    unsafe { argument_field.write_volatile(argument) };
    // SAFETY: `token` aponta para o `MpInfo` do núcleo; `goto_address` é atômico.
    let goto = unsafe { &(*info).goto_address };
    // O release publica o argumento antes do salto.
    goto.store(entry as *mut (), Ordering::Release);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootError {
    UnsupportedBaseRevision,
    MissingMemoryMap,
    MissingDirectMap,
    MissingKernelAddress,
}

/// Visão do kernel sobre as respostas do Limine. Só existe se o essencial chegou.
#[derive(Debug)]
pub struct LimineBoot {
    entries: &'static [&'static MemmapEntry],
    cpus: &'static [&'static MpInfo],
    bsp_hw_id: u64,
    direct_map_offset: u64,
    kernel_image: KernelImage,
}

impl LimineBoot {
    /// # Safety
    /// Chamar uma vez, na entrada do kernel, antes de qualquer escrita na memória
    /// marcada como `BootloaderReclaimable` (onde moram as respostas).
    pub unsafe fn from_bootloader() -> Result<Self, BootError> {
        if !requests::base_revision_supported() {
            return Err(BootError::UnsupportedBaseRevision);
        }
        // SAFETY: repassada a quem chama; as respostas ainda estão intactas.
        let memmap = unsafe { requests::memmap() }.ok_or(BootError::MissingMemoryMap)?;
        // SAFETY: idem.
        let hhdm = unsafe { requests::hhdm() }.ok_or(BootError::MissingDirectMap)?;
        // SAFETY: idem.
        let address =
            unsafe { requests::executable_address() }.ok_or(BootError::MissingKernelAddress)?;
        // SAFETY: o Limine garante `entry_count` ponteiros válidos em `entries`.
        let entries = unsafe { memmap.entries() };
        // SAFETY: as respostas ainda estão intactas; sem resposta de MP o kernel roda numa CPU.
        let mp = unsafe { requests::mp() };
        // SAFETY: o Limine garante `cpu_count` ponteiros válidos em `cpus`.
        let cpus = mp.map_or(&[][..], |mp| unsafe { mp.cpus() });
        Ok(Self {
            entries,
            cpus,
            bsp_hw_id: mp.map_or(0, protocol::MpResponse::bsp_hw_id),
            direct_map_offset: hhdm.offset,
            kernel_image: KernelImage {
                physical_base: PhysAddr::new(address.physical_base),
                virtual_base: VirtAddr::new(address.virtual_base),
            },
        })
    }
}

impl LimineBoot {
    /// A linha de comando da entrada do `limine.conf`, se ela tem uma e é ASCII válido.
    ///
    /// # Safety
    /// A memória das respostas (`BootloaderReclaimable`) ainda não foi reaproveitada.
    pub unsafe fn command_line(&self) -> Option<&str> {
        // SAFETY: repassada a quem chama.
        let response = unsafe { requests::executable_cmdline() }?;
        // SAFETY: o Limine entrega string C terminada em zero, na memória das respostas.
        unsafe { c_str(response.cmdline) }
    }
}

impl LimineBoot {
    /// A faixa física do framebuffer, em páginas inteiras (como o kernel a mapeia).
    fn framebuffer_pages(&self) -> Option<(u64, u64)> {
        let info = self.framebuffer()?;
        let start = info.address.as_u64().checked_sub(self.direct_map_offset)?;
        let end = start.checked_add(info.len_bytes())?;
        Some((
            start & !(PAGE_SIZE - 1),
            end.checked_next_multiple_of(PAGE_SIZE)?,
        ))
    }
}

/// `is_boot` de um núcleo: o que tem o identificador do BSP que a resposta informou.
fn cpu_descriptor(info: &MpInfo, bsp_hw_id: u64) -> CpuDescriptor {
    CpuDescriptor {
        hw_id: info.hw_id(),
        is_boot: info.hw_id() == bsp_hw_id,
        token: ptr::from_ref(info).addr(),
    }
}

impl BootInfo for LimineBoot {
    fn memory_regions(&self) -> impl Iterator<Item = MemoryRegion> + '_ {
        self.entries.iter().map(|entry| MemoryRegion {
            start: PhysAddr::new(entry.base),
            len: entry.length,
            kind: region_kind(entry.kind),
        })
    }

    fn direct_map_offset(&self) -> u64 {
        self.direct_map_offset
    }

    fn bootloader(&self) -> Option<BootloaderId<'_>> {
        // SAFETY: as respostas seguem válidas enquanto a memória do bootloader não é reaproveitada.
        let info = unsafe { requests::bootloader_info() }?;
        // SAFETY: o Limine entrega strings C terminadas em zero.
        let name = unsafe { c_str(info.name) }?;
        // SAFETY: idem.
        let version = unsafe { c_str(info.version) }?;
        Some(BootloaderId { name, version })
    }

    fn timeline(&self) -> Option<BootTimeline> {
        // SAFETY: as respostas seguem válidas enquanto a memória do bootloader não é reaproveitada.
        let perf = unsafe { requests::performance() }?;
        Some(BootTimeline {
            reset_us: perf.reset_usec,
            loader_start_us: perf.init_usec,
            kernel_handoff_us: perf.exec_usec,
        })
    }

    fn kernel_image(&self) -> Option<KernelImage> {
        Some(self.kernel_image)
    }

    fn cpus(&self) -> impl Iterator<Item = CpuDescriptor> + '_ {
        let bsp = self.bsp_hw_id;
        self.cpus.iter().map(move |info| cpu_descriptor(info, bsp))
    }

    fn modules(&self) -> impl Iterator<Item = BootModule<'_>> + '_ {
        // SAFETY: as respostas seguem válidas enquanto a memória do bootloader não é reaproveitada.
        let response = unsafe { requests::modules() };
        // SAFETY: o Limine garante `module_count` ponteiros, nulos ou de `limine_file` válidos.
        let files = response.into_iter().flat_map(|r| unsafe { r.files() });
        let framebuffer = self.framebuffer_pages();
        let mut accepted = [(0, 0); protocol::MAX_MODULES];
        let mut count = 0;
        files.filter_map(move |file| {
            let limits = Limits {
                hhdm: self.direct_map_offset,
                entries: self.entries,
                framebuffer,
                accepted: &accepted[..count],
            };
            let module = module_info(file, &limits)?;
            let start = module.start.as_u64();
            *accepted.get_mut(count)? = (start, start + module.len);
            count += 1;
            Some(module)
        })
    }

    fn framebuffer(&self) -> Option<FramebufferInfo> {
        // SAFETY: as respostas seguem válidas enquanto a memória do bootloader não é reaproveitada.
        let response = unsafe { requests::framebuffer() }?;
        // SAFETY: o Limine garante `framebuffer_count` ponteiros válidos.
        let framebuffers = unsafe { response.framebuffers() };
        framebuffers.first().and_then(|fb| framebuffer_info(fb))
    }
}

/// O que um módulo não pode tocar nem passar, tudo em endereço físico.
struct Limits<'a> {
    hhdm: u64,
    entries: &'a [&'a MemmapEntry],
    /// O framebuffer, `[início, fim)` em páginas inteiras.
    framebuffer: Option<(u64, u64)>,
    /// Os módulos já aceitos, `[início, fim)`.
    accepted: &'a [(u64, u64)],
}

/// Um módulo do Limine para o do núcleo. `None` se a faixa não é a de um arquivo carregado.
fn module_info(file: &File, limits: &Limits<'_>) -> Option<BootModule<'static>> {
    let (start, len) = module_range(file.address.addr() as u64, file.size, limits)?;
    // SAFETY: `string` é nula ou aponta para memória das respostas; a leitura para no NUL ou
    // depois de `MODULE_STRING_MAX + 1` bytes.
    let string = unsafe { bounded_c_str(file.string, MODULE_STRING_MAX) };
    Some(BootModule {
        start: PhysAddr::new(start),
        len,
        string: string.unwrap_or(""),
    })
}

fn overlaps(a: (u64, u64), b: (u64, u64)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

/// A faixa física `(início, tamanho)` de um módulo, do endereço no HHDM que o Limine deu, ou
/// `None` pelas regras do SDD 04. A sobreposição com a imagem do kernel é do `Vm`.
fn module_range(virt: u64, size: u64, limits: &Limits<'_>) -> Option<(u64, u64)> {
    if virt == 0 || size == 0 || size > MODULE_MAX_BYTES {
        return None;
    }
    let start = virt.checked_sub(limits.hhdm)?;
    let end = start.checked_add(size)?;
    let memory_end = limits
        .entries
        .iter()
        .map(|entry| entry.base.saturating_add(entry.length))
        .max()?;
    if !start.is_multiple_of(PAGE_SIZE) || end > memory_end {
        return None;
    }
    // Páginas inteiras: o Limine dá ao módulo todos os pedaços de 4 KiB que ele cobre.
    let pages = (start, end.checked_next_multiple_of(PAGE_SIZE)?);
    let touches_ram = limits.entries.iter().any(|entry| {
        let ram = matches!(
            entry.kind,
            protocol::MEMMAP_USABLE | protocol::MEMMAP_BOOTLOADER_RECLAIMABLE
        );
        ram && overlaps(pages, (entry.base, entry.base.saturating_add(entry.length)))
    });
    let touches_other = limits
        .framebuffer
        .iter()
        .chain(limits.accepted)
        .any(|&taken| overlaps(pages, taken));
    (!touches_ram && !touches_other).then_some((start, size))
}

/// Só 32 bpp em RGB com canais de 8 bits; outro formato fica sem tela.
fn framebuffer_info(fb: &Framebuffer) -> Option<FramebufferInfo> {
    let channel =
        |size: u8, shift: u8| (size == 8 && shift <= 24).then_some(Channel { shift, size });
    if fb.memory_model != protocol::FRAMEBUFFER_RGB || fb.bpp != 32 || fb.address.is_null() {
        return None;
    }
    let info = FramebufferInfo {
        address: VirtAddr::new(fb.address.addr() as u64),
        width: u32::try_from(fb.width).ok()?,
        height: u32::try_from(fb.height).ok()?,
        pitch: u32::try_from(fb.pitch).ok()?,
        red: channel(fb.red_mask_size, fb.red_mask_shift)?,
        green: channel(fb.green_mask_size, fb.green_mask_shift)?,
        blue: channel(fb.blue_mask_size, fb.blue_mask_shift)?,
    };
    let fits = info.pitch.is_multiple_of(FramebufferInfo::BYTES_PER_PIXEL)
        && info.stride_pixels() >= info.width;
    (fits && info.width > 0 && info.height > 0).then_some(info)
}

/// # Safety
/// `ptr` é nulo ou aponta para uma string C válida por `'a`.
unsafe fn c_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: não nulo e terminado em zero, pelo contrato da função.
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}

/// Lê uma string C de até `max` bytes mais o NUL, sem passar dessa janela; `None` se nula, sem
/// NUL nela ou que não é UTF-8.
/// # Safety
/// `ptr` é nulo ou os bytes até o NUL (sem NUL, os `max + 1` primeiros) são legíveis por `'a`.
unsafe fn bounded_c_str<'a>(ptr: *const c_char, max: usize) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    let bytes = ptr.cast::<u8>();
    let mut len = 0;
    // SAFETY: a posição `len` está na janela legível, pelo contrato da função.
    while unsafe { bytes.wrapping_add(len).read() } != 0 {
        len += 1;
        if len > max {
            return None;
        }
    }
    // SAFETY: os `len` bytes lidos acima são legíveis, pelo contrato da função.
    let text = unsafe { slice::from_raw_parts(bytes, len) };
    str::from_utf8(text).ok()
}

/// Tipo de região do Limine para o do núcleo. Tipo desconhecido é tratado como reservado.
fn region_kind(raw: u64) -> RegionKind {
    match raw {
        protocol::MEMMAP_USABLE => RegionKind::Usable,
        protocol::MEMMAP_ACPI_RECLAIMABLE => RegionKind::AcpiReclaimable,
        protocol::MEMMAP_ACPI_NVS => RegionKind::AcpiNvs,
        protocol::MEMMAP_BAD_MEMORY => RegionKind::BadMemory,
        protocol::MEMMAP_BOOTLOADER_RECLAIMABLE => RegionKind::BootloaderReclaimable,
        protocol::MEMMAP_EXECUTABLE_AND_MODULES => RegionKind::KernelAndModules,
        protocol::MEMMAP_FRAMEBUFFER => RegionKind::Framebuffer,
        _ => RegionKind::Reserved,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_known_region_type() {
        assert_eq!(region_kind(0), RegionKind::Usable);
        assert_eq!(region_kind(2), RegionKind::AcpiReclaimable);
        assert_eq!(region_kind(3), RegionKind::AcpiNvs);
        assert_eq!(region_kind(4), RegionKind::BadMemory);
        assert_eq!(region_kind(5), RegionKind::BootloaderReclaimable);
        assert_eq!(region_kind(6), RegionKind::KernelAndModules);
        assert_eq!(region_kind(7), RegionKind::Framebuffer);
    }

    #[test]
    fn reserved_and_unknown_types_are_reserved() {
        assert_eq!(region_kind(1), RegionKind::Reserved);
        assert_eq!(region_kind(8), RegionKind::Reserved);
        assert_eq!(region_kind(999), RegionKind::Reserved);
    }

    fn limine_fb(bpp: u16, pitch: u64) -> Framebuffer {
        Framebuffer {
            address: core::ptr::without_provenance_mut(0xffff_8000_fd00_0000),
            width: 1280,
            height: 800,
            pitch,
            bpp,
            memory_model: protocol::FRAMEBUFFER_RGB,
            red_mask_size: 8,
            red_mask_shift: 16,
            green_mask_size: 8,
            green_mask_shift: 8,
            blue_mask_size: 8,
            blue_mask_shift: 0,
            unused: [0; 7],
            edid_size: 0,
            edid: core::ptr::null(),
        }
    }

    #[test]
    fn translates_rgb_32bpp_framebuffer() {
        let info = framebuffer_info(&limine_fb(32, 1280 * 4)).expect("formato aceito");
        assert_eq!(
            (info.width, info.height, info.stride_pixels()),
            (1280, 800, 1280)
        );
        assert_eq!(info.red, Channel { shift: 16, size: 8 });
        assert_eq!(info.address, VirtAddr::new(0xffff_8000_fd00_0000));
    }

    #[test]
    fn rejects_formats_the_renderer_cannot_draw() {
        assert!(framebuffer_info(&limine_fb(24, 1280 * 3)).is_none());
        assert!(framebuffer_info(&limine_fb(32, 1279 * 4)).is_none());
        assert!(framebuffer_info(&limine_fb(32, 1280 * 4 + 2)).is_none());
        let mut fb = limine_fb(32, 1280 * 4);
        fb.memory_model = 0;
        assert!(framebuffer_info(&fb).is_none());
        let mut fb = limine_fb(32, 1280 * 4);
        fb.green_mask_size = 6;
        assert!(framebuffer_info(&fb).is_none());
    }

    #[test]
    fn marks_only_the_cpu_with_the_bsp_id_as_boot() {
        let boot = protocol::test_info(3);
        let other = protocol::test_info(5);
        assert!(cpu_descriptor(&boot, 3).is_boot);
        assert!(!cpu_descriptor(&other, 3).is_boot);
        assert_eq!(cpu_descriptor(&other, 3).hw_id, 5);
        assert_eq!(
            cpu_descriptor(&other, 3).token,
            ptr::from_ref(&other).addr()
        );
    }

    const HHDM: u64 = 0xffff_8000_0000_0000;
    /// Fim do mapa de memória do teste.
    const MEMORY_END: u64 = 0x1_0000_0000;

    fn entry(base: u64, length: u64, kind: u64) -> MemmapEntry {
        MemmapEntry { base, length, kind }
    }

    /// O mapa do teste: RAM de 0 a 2 MiB, módulos em `0x40_0000`, RAM de `0x80_0000` a 24 MiB, uma
    /// faixa `Reserved` que sobrepõe o módulo e o fim em `MEMORY_END`.
    fn map() -> [MemmapEntry; 5] {
        [
            entry(0, 0x20_0000, protocol::MEMMAP_USABLE),
            entry(
                0x40_0000,
                0x40_0000,
                protocol::MEMMAP_EXECUTABLE_AND_MODULES,
            ),
            entry(0x50_0000, 0x1000, 1),
            entry(
                0x80_0000,
                0x100_0000,
                protocol::MEMMAP_BOOTLOADER_RECLAIMABLE,
            ),
            entry(MEMORY_END - 0x1000, 0x1000, protocol::MEMMAP_ACPI_NVS),
        ]
    }

    /// Módulo de `size` bytes em `phys`, com framebuffer em `[0x70_0000, 0x70_8000)` e um módulo
    /// já aceito em `[0x60_0000, 0x60_1000)`.
    fn range(phys: u64, size: u64) -> Option<(u64, u64)> {
        let map = map();
        let entries: Vec<&MemmapEntry> = map.iter().collect();
        let limits = Limits {
            hhdm: HHDM,
            entries: &entries,
            framebuffer: Some((0x70_0000, 0x70_8000)),
            accepted: &[(0x60_0000, 0x60_1000)],
        };
        module_range(HHDM + phys, size, &limits)
    }

    #[test]
    fn accepts_a_page_aligned_module_outside_ram() {
        assert_eq!(range(0x40_0000, 512), Some((0x40_0000, 512)));
        assert_eq!(range(0x7f_f000, 0x1000), Some((0x7f_f000, 0x1000)));
    }

    #[test]
    fn a_reserved_or_module_entry_under_the_module_is_not_a_reason_to_refuse() {
        assert_eq!(range(0x50_0000, 0x1000), Some((0x50_0000, 0x1000)));
    }

    #[test]
    fn rejects_module_addresses_the_bootloader_cannot_have_given() {
        let limits = Limits {
            hhdm: HHDM,
            entries: &[],
            framebuffer: None,
            accepted: &[],
        };
        assert_eq!(module_range(0, 512, &limits), None);
        assert_eq!(module_range(HHDM - 0x1000, 512, &limits), None);
        assert_eq!(range(0x40_0010, 512), None);
    }

    #[test]
    fn rejects_empty_or_overflowing_modules() {
        assert_eq!(range(0x40_0000, 0), None);
        assert_eq!(range(0x40_0000, u64::MAX), None);
        let map = map();
        let entries: Vec<&MemmapEntry> = map.iter().collect();
        let limits = Limits {
            hhdm: 0,
            entries: &entries,
            framebuffer: None,
            accepted: &[],
        };
        assert_eq!(module_range(!0xfff, 0x2000, &limits), None);
    }

    #[test]
    fn rejects_a_module_that_ends_past_the_memory_map() {
        assert_eq!(range(MEMORY_END, 0x1000), None);
        assert_eq!(range(MEMORY_END - 0x1000, 0x2000), None);
        assert_eq!(range(1 << 40, 0x1000), None);
        let map = [entry(0, 0x20_0000, protocol::MEMMAP_USABLE)];
        let entries: Vec<&MemmapEntry> = map.iter().collect();
        let limits = Limits {
            hhdm: HHDM,
            entries: &entries,
            framebuffer: None,
            accepted: &[],
        };
        assert_eq!(module_range(HHDM + 0x20_0000, 0x1000, &limits), None);
    }

    #[test]
    fn rejects_a_module_above_the_window_size() {
        assert_eq!(range(0x1_0000, MODULE_MAX_BYTES + 1), None);
        let map = [entry(
            0x40_0000,
            4 << 30,
            protocol::MEMMAP_EXECUTABLE_AND_MODULES,
        )];
        let entries: Vec<&MemmapEntry> = map.iter().collect();
        let limits = Limits {
            hhdm: HHDM,
            entries: &entries,
            framebuffer: None,
            accepted: &[],
        };
        let at = |size| module_range(HHDM + 0x40_0000, size, &limits);
        assert_eq!(at(MODULE_MAX_BYTES), Some((0x40_0000, MODULE_MAX_BYTES)));
        assert_eq!(at(MODULE_MAX_BYTES + 1), None);
    }

    #[test]
    fn rejects_a_module_that_overlaps_allocator_memory() {
        assert_eq!(range(0x1f_f000, 0x2000), None);
        assert_eq!(range(0x7f_f000, 0x2000), None);
        assert_eq!(range(0x10_0000, 0x1000), None);
    }

    #[test]
    fn rejects_a_module_that_overlaps_the_framebuffer() {
        assert_eq!(range(0x6f_f000, 0x2000), None);
        assert_eq!(range(0x70_7000, 0x1000), None);
        assert_eq!(range(0x70_8000, 0x1000), Some((0x70_8000, 0x1000)));
    }

    #[test]
    fn rejects_a_module_that_overlaps_one_already_accepted() {
        assert_eq!(range(0x5f_f000, 0x2000), None);
        assert_eq!(range(0x60_0000, 0x1000), None);
        assert_eq!(range(0x60_1000, 0x1000), Some((0x60_1000, 0x1000)));
    }

    #[test]
    fn a_module_shorter_than_a_page_owns_the_whole_page() {
        assert_eq!(range(0x5f_f000, 1), Some((0x5f_f000, 1)));
        assert_eq!(range(0x5f_f000, 0x1001), None);
    }

    fn file(address: u64, size: u64, string: &'static core::ffi::CStr) -> File {
        protocol::test_file(address, size, string.as_ptr())
    }

    fn with_limits<T>(run: impl FnOnce(&Limits<'_>) -> T) -> T {
        let map = map();
        let entries: Vec<&MemmapEntry> = map.iter().collect();
        run(&Limits {
            hhdm: HHDM,
            entries: &entries,
            framebuffer: None,
            accepted: &[],
        })
    }

    #[test]
    fn translates_a_module_with_its_string() {
        let module =
            with_limits(|limits| module_info(&file(HHDM + 0x40_0000, 100, c"initramfs"), limits));
        let module = module.expect("módulo válido");
        assert_eq!((module.start, module.len), (PhysAddr::new(0x40_0000), 100));
        assert_eq!(module.string, "initramfs");
    }

    #[test]
    fn a_module_string_that_is_null_or_too_long_is_empty() {
        let string_of = |file: &File| {
            with_limits(|limits| module_info(file, limits))
                .unwrap()
                .string
        };
        let mut null = file(HHDM + 0x40_0000, 100, c"");
        null.string = core::ptr::null();
        assert_eq!(string_of(&null), "");
        let long = c"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\
            0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\
            0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\
            0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0";
        assert_eq!(long.count_bytes(), MODULE_STRING_MAX + 1);
        assert_eq!(string_of(&file(HHDM + 0x40_0000, 1, long)), "");
    }

    #[test]
    fn a_module_string_that_is_not_utf8_is_empty() {
        let bad = file(HHDM + 0x40_0000, 1, c"init\xfframfs");
        assert_eq!(
            with_limits(|limits| module_info(&bad, limits))
                .unwrap()
                .string,
            ""
        );
    }

    #[test]
    fn a_string_without_nul_is_read_only_up_to_the_window() {
        // Exatamente `MODULE_STRING_MAX + 1` bytes e nenhum zero: a leitura não pode passar daqui.
        let bytes = [b'a'; MODULE_STRING_MAX + 1];
        // SAFETY: o vetor tem os `max + 1` bytes que a leitura pode tocar.
        let read = unsafe { bounded_c_str(bytes.as_ptr().cast(), MODULE_STRING_MAX) };
        assert_eq!(read, None);
    }

    #[test]
    fn a_string_that_fills_the_window_with_its_nul_is_kept() {
        let mut bytes = [b'a'; MODULE_STRING_MAX + 1];
        bytes[MODULE_STRING_MAX] = 0;
        // SAFETY: idem; o NUL está na última posição da janela.
        let read = unsafe { bounded_c_str(bytes.as_ptr().cast(), MODULE_STRING_MAX) };
        assert_eq!(read.map(str::len), Some(MODULE_STRING_MAX));
    }

    #[test]
    fn null_c_string_is_none() {
        // SAFETY: ponteiro nulo é aceito pelo contrato.
        assert_eq!(unsafe { c_str(core::ptr::null()) }, None);
    }

    #[test]
    fn a_command_line_that_is_not_utf8_drops_every_parameter() {
        let raw = c"rustos.selftest=1 \xff";
        // SAFETY: literal C terminado em zero e estático.
        let line = unsafe { c_str(raw.as_ptr()) };
        assert_eq!(line, None);
        let params = rustos_hal::BootParams::parse(line.unwrap_or(""));
        assert_eq!(params, rustos_hal::BootParams::default());
    }

    #[test]
    fn c_string_is_read_until_nul() {
        let raw = c"Limine";
        // SAFETY: literal C terminado em zero e estático.
        assert_eq!(unsafe { c_str(raw.as_ptr()) }, Some("Limine"));
    }
}
