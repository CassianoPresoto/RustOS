//! Adaptador do protocolo de boot Limine (revisão base 6): traduz as respostas
//! do bootloader para `hal::BootInfo`. Protocolo escrito aqui, sem crate externo.

#![cfg_attr(not(test), no_std)]

mod protocol;
mod requests;

use core::ffi::{CStr, c_char};
use core::ptr;
use core::sync::atomic::Ordering;

use rustos_hal::{
    BootInfo, BootModule, BootTimeline, BootloaderId, Channel, CpuDescriptor, FramebufferInfo,
    KernelImage, MemoryRegion, PAGE_SIZE, PhysAddr, RegionKind, VirtAddr,
};

pub use protocol::{BASE_REVISION, MpInfo};
use protocol::{File, Framebuffer, MemmapEntry};

/// Maior endereço físico que as duas ISAs endereçam (52 bits).
const PHYS_LIMIT: u64 = 1 << 52;
/// Maior `string` de módulo que o kernel guarda; uma maior vira vazia.
const MODULE_STRING_MAX: usize = 256;

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
        files.filter_map(|file| module_info(file, self.direct_map_offset, self.entries))
    }

    fn framebuffer(&self) -> Option<FramebufferInfo> {
        // SAFETY: as respostas seguem válidas enquanto a memória do bootloader não é reaproveitada.
        let response = unsafe { requests::framebuffer() }?;
        // SAFETY: o Limine garante `framebuffer_count` ponteiros válidos.
        let framebuffers = unsafe { response.framebuffers() };
        framebuffers.first().and_then(|fb| framebuffer_info(fb))
    }
}

/// Um módulo do Limine para o do núcleo. `None` se a faixa não é a de um arquivo carregado.
fn module_info(file: &File, hhdm: u64, entries: &[&MemmapEntry]) -> Option<BootModule<'static>> {
    let (start, len) = module_range(file.address.addr() as u64, file.size, hhdm, entries)?;
    // SAFETY: o Limine entrega `string` nula ou C terminada em zero, na memória das respostas.
    let string = unsafe { c_str(file.string) };
    Some(BootModule {
        start: PhysAddr::new(start),
        len,
        string: string
            .filter(|text| text.len() <= MODULE_STRING_MAX)
            .unwrap_or(""),
    })
}

/// A faixa física `(início, tamanho)` de um módulo, a partir do endereço no HHDM que o Limine
/// deu. Recusa endereço nulo ou abaixo do HHDM, desalinhado, tamanho zero, faixa que transborda
/// ou passa de 52 bits, e faixa que toca RAM do alocador (o Limine promete que não toca).
fn module_range(virt: u64, size: u64, hhdm: u64, entries: &[&MemmapEntry]) -> Option<(u64, u64)> {
    if virt == 0 || size == 0 {
        return None;
    }
    let start = virt.checked_sub(hhdm)?;
    let end = start.checked_add(size)?;
    if !start.is_multiple_of(PAGE_SIZE) || end > PHYS_LIMIT {
        return None;
    }
    let touches_ram = entries.iter().any(|entry| {
        let ram = matches!(
            entry.kind,
            protocol::MEMMAP_USABLE | protocol::MEMMAP_BOOTLOADER_RECLAIMABLE
        );
        ram && entry.base < end && start < entry.base.saturating_add(entry.length)
    });
    (!touches_ram).then_some((start, size))
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

    fn entry(base: u64, length: u64, kind: u64) -> MemmapEntry {
        MemmapEntry { base, length, kind }
    }

    /// Módulo de `size` bytes em `phys`, entre RAM `Usable` antes e depois.
    fn range(phys: u64, size: u64) -> Option<(u64, u64)> {
        let ram = [
            entry(0, 0x20_0000, protocol::MEMMAP_USABLE),
            entry(
                0x80_0000,
                0x100_0000,
                protocol::MEMMAP_BOOTLOADER_RECLAIMABLE,
            ),
            entry(
                0x40_0000,
                0x40_0000,
                protocol::MEMMAP_EXECUTABLE_AND_MODULES,
            ),
        ];
        module_range(HHDM + phys, size, HHDM, &ram.each_ref().map(|entry| entry))
    }

    #[test]
    fn accepts_a_page_aligned_module_outside_ram() {
        assert_eq!(range(0x40_0000, 512), Some((0x40_0000, 512)));
        assert_eq!(range(0x7f_f000, 0x1000), Some((0x7f_f000, 0x1000)));
    }

    #[test]
    fn rejects_module_addresses_the_bootloader_cannot_have_given() {
        assert_eq!(module_range(0, 512, HHDM, &[]), None);
        assert_eq!(module_range(HHDM - 0x1000, 512, HHDM, &[]), None);
        assert_eq!(range(0x40_0010, 512), None);
    }

    #[test]
    fn rejects_empty_or_overflowing_modules() {
        assert_eq!(range(0x40_0000, 0), None);
        assert_eq!(range(0x40_0000, u64::MAX), None);
        assert_eq!(range(0x40_0000, PHYS_LIMIT), None);
        assert_eq!(module_range(!0xfff, 0x2000, 0, &[]), None);
    }

    #[test]
    fn rejects_a_module_that_overlaps_allocator_memory() {
        assert_eq!(range(0x1f_f000, 0x2000), None);
        assert_eq!(range(0x7f_f000, 0x2000), None);
        assert_eq!(range(0x10_0000, 0x1000), None);
    }

    fn file(address: u64, size: u64, string: &'static core::ffi::CStr) -> File {
        protocol::test_file(address, size, string.as_ptr())
    }

    #[test]
    fn translates_a_module_with_its_string() {
        let module = module_info(&file(HHDM + 0x40_0000, 100, c"initramfs"), HHDM, &[]);
        let module = module.expect("módulo válido");
        assert_eq!((module.start, module.len), (PhysAddr::new(0x40_0000), 100));
        assert_eq!(module.string, "initramfs");
    }

    #[test]
    fn a_module_string_that_is_null_or_too_long_is_empty() {
        let mut null = file(HHDM + 0x40_0000, 100, c"");
        null.string = core::ptr::null();
        assert_eq!(module_info(&null, HHDM, &[]).unwrap().string, "");
        let long = c"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\
            0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\
            0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\
            0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0";
        assert_eq!(long.count_bytes(), MODULE_STRING_MAX + 1);
        assert_eq!(
            module_info(&file(HHDM + 0x40_0000, 1, long), HHDM, &[])
                .unwrap()
                .string,
            ""
        );
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
