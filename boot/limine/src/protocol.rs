//! Estruturas do protocolo Limine, campo a campo como em `limine.h` (limine-protocol).

use core::cell::UnsafeCell;
use core::ffi::c_char;
use core::sync::atomic::AtomicPtr;
use core::{ptr, slice};

pub const BASE_REVISION: u64 = 6;

const COMMON_MAGIC: [u64; 2] = [0xc7b1_dd30_df4c_8b88, 0x0a82_e883_a194_f07b];
const BASE_REVISION_MAGIC: [u64; 2] = [0xf956_2b2d_5c95_a6c8, 0x6a7b_3849_4453_6bdc];

pub const REQUESTS_START_MARKER: [u64; 4] = [
    0xf6b8_f4b3_9de7_d1ae,
    0xfab9_1a69_40fc_b9cf,
    0x785c_6ed0_15d3_e316,
    0x181e_920a_7852_b9d9,
];
pub const REQUESTS_END_MARKER: [u64; 2] = [0xadc0_e053_1bb1_0d03, 0x9572_709f_3176_4c62];

pub const BOOTLOADER_INFO_ID: [u64; 4] = id(0xf550_38d8_e2a1_202f, 0x2794_26fc_f5f5_9740);
pub const HHDM_ID: [u64; 4] = id(0x48dc_f1cb_8ad2_b852, 0x6398_4e95_9a98_244b);
pub const EXECUTABLE_CMDLINE_ID: [u64; 4] = id(0x4b16_1536_e598_651e, 0xb390_ad4a_2f1f_303a);
pub const MEMMAP_ID: [u64; 4] = id(0x67cf_3d9d_378a_806f, 0xe304_acdf_c50c_3c62);
pub const PERFORMANCE_ID: [u64; 4] = id(0x6b50_ad9b_f36d_13ad, 0xdc4c_7e88_fc75_9e17);
pub const FRAMEBUFFER_ID: [u64; 4] = id(0x9d58_27dc_d881_dd75, 0xa314_8604_f6fa_b11b);
pub const EXECUTABLE_ADDRESS_ID: [u64; 4] = id(0x71ba_7686_3cc5_5f63, 0xb264_4a48_c516_a487);
pub const MODULE_ID: [u64; 4] = id(0x3e7e_2797_02be_32af, 0xca1c_4f3b_d128_0cee);

pub const MP_ID: [u64; 4] = id(0x95a6_7b81_9a1b_857e, 0xa0b6_1b72_3b6a_73e0);
/// `LIMINE_MP_REQUEST_X86_64_X2APIC`: pede ao bootloader o x2APIC ligado, se a CPU o tem.
pub const MP_REQUEST_X2APIC: u64 = 1;

pub const FRAMEBUFFER_RGB: u8 = 1;

pub const MEMMAP_USABLE: u64 = 0;
pub const MEMMAP_ACPI_RECLAIMABLE: u64 = 2;
pub const MEMMAP_ACPI_NVS: u64 = 3;
pub const MEMMAP_BAD_MEMORY: u64 = 4;
pub const MEMMAP_BOOTLOADER_RECLAIMABLE: u64 = 5;
pub const MEMMAP_EXECUTABLE_AND_MODULES: u64 = 6;
pub const MEMMAP_FRAMEBUFFER: u64 = 7;

const fn id(a: u64, b: u64) -> [u64; 4] {
    [COMMON_MAGIC[0], COMMON_MAGIC[1], a, b]
}

/// O bootloader zera a terceira palavra quando aceita a revisão pedida.
#[repr(C, align(8))]
pub struct BaseRevision(UnsafeCell<[u64; 3]>);

// SAFETY: só o bootloader escreve, antes de o kernel rodar; depois é só leitura.
unsafe impl Sync for BaseRevision {}

impl BaseRevision {
    pub const fn new(revision: u64) -> Self {
        Self(UnsafeCell::new([
            BASE_REVISION_MAGIC[0],
            BASE_REVISION_MAGIC[1],
            revision,
        ]))
    }

    pub fn is_supported(&self) -> bool {
        // SAFETY: leitura volátil de memória que só o bootloader escreveu.
        let words = unsafe { ptr::read_volatile(self.0.get()) };
        words[2] == 0
    }
}

#[repr(C, align(8))]
pub struct Marker<const N: usize>([u64; N]);

impl<const N: usize> Marker<N> {
    pub const fn new(words: [u64; N]) -> Self {
        Self(words)
    }
}

/// Pedido ao bootloader; ele preenche `response` antes de entrar no kernel.
#[repr(C, align(8))]
pub struct Request<R> {
    id: [u64; 4],
    revision: u64,
    response: UnsafeCell<*const R>,
}

// SAFETY: só o bootloader escreve `response`, antes de o kernel rodar; depois é só leitura.
unsafe impl<R> Sync for Request<R> {}

impl<R> Request<R> {
    pub const fn new(id: [u64; 4]) -> Self {
        Self {
            id,
            revision: 0,
            response: UnsafeCell::new(ptr::null()),
        }
    }

    /// # Safety
    /// A memória das respostas (`BootloaderReclaimable`) ainda não foi reaproveitada.
    pub unsafe fn response(&self) -> Option<&'static R> {
        // SAFETY: leitura volátil: o valor foi escrito pelo bootloader, fora da visão do compilador.
        let response = unsafe { ptr::read_volatile(self.response.get()) };
        // SAFETY: não nulo é um ponteiro válido do bootloader, pelo contrato da função.
        unsafe { response.as_ref() }
    }
}

/// Pedido de MP: o de sempre mais `flags`, depois da resposta (`struct limine_mp_request`).
#[repr(C, align(8))]
pub struct MpRequest {
    id: [u64; 4],
    revision: u64,
    response: UnsafeCell<*const MpResponse>,
    flags: u64,
}

// SAFETY: só o bootloader escreve `response`, antes de o kernel rodar; depois é só leitura.
unsafe impl Sync for MpRequest {}

impl MpRequest {
    pub const fn new(flags: u64) -> Self {
        Self {
            id: MP_ID,
            revision: 0,
            response: UnsafeCell::new(ptr::null()),
            flags,
        }
    }

    /// # Safety
    /// A memória das respostas (`BootloaderReclaimable`) ainda não foi reaproveitada.
    pub unsafe fn response(&self) -> Option<&'static MpResponse> {
        // SAFETY: leitura volátil: o valor foi escrito pelo bootloader, fora da visão do compilador.
        let response = unsafe { ptr::read_volatile(self.response.get()) };
        // SAFETY: não nulo é um ponteiro válido do bootloader, pelo contrato da função.
        unsafe { response.as_ref() }
    }
}

/// `struct limine_mp_info` do x86_64.
#[derive(Debug)]
#[allow(dead_code, reason = "o layout da outra ISA só entra no teste do host")]
#[repr(C)]
pub struct MpInfoX86 {
    pub processor_id: u32,
    pub lapic_id: u32,
    reserved: u64,
    pub goto_address: AtomicPtr<()>,
    pub extra_argument: u64,
}

/// `struct limine_mp_info` do aarch64.
#[derive(Debug)]
#[allow(dead_code, reason = "o layout da outra ISA só entra no teste do host")]
#[repr(C)]
pub struct MpInfoAarch64 {
    pub processor_id: u32,
    reserved1: u32,
    pub mpidr: u64,
    reserved: u64,
    pub goto_address: AtomicPtr<()>,
    pub extra_argument: u64,
}

/// `struct limine_mp_response` do x86_64.
#[allow(dead_code, reason = "o layout da outra ISA só entra no teste do host")]
#[repr(C)]
pub struct MpResponseX86 {
    pub revision: u64,
    pub flags: u32,
    pub bsp_lapic_id: u32,
    cpu_count: u64,
    cpus: *const *const MpInfoX86,
}

/// `struct limine_mp_response` do aarch64.
#[allow(dead_code, reason = "o layout da outra ISA só entra no teste do host")]
#[repr(C)]
pub struct MpResponseAarch64 {
    pub revision: u64,
    pub flags: u64,
    pub bsp_mpidr: u64,
    cpu_count: u64,
    cpus: *const *const MpInfoAarch64,
}

#[cfg(not(target_arch = "x86_64"))]
pub use {MpInfoAarch64 as MpInfo, MpResponseAarch64 as MpResponse};
#[cfg(target_arch = "x86_64")]
pub use {MpInfoX86 as MpInfo, MpResponseX86 as MpResponse};

impl MpInfo {
    /// O `extra_argument` que o BSP gravou antes de soltar este núcleo.
    pub fn argument(&self) -> u64 {
        // SAFETY: o campo está dentro de `self`; o BSP o escreveu antes do `goto_address`.
        unsafe { ptr::read_volatile(&raw const self.extra_argument) }
    }

    /// Identificador de hardware: LAPIC ID ou afinidade do `MPIDR`.
    pub fn hw_id(&self) -> u64 {
        #[cfg(target_arch = "x86_64")]
        return u64::from(self.lapic_id);
        #[cfg(not(target_arch = "x86_64"))]
        return self.mpidr;
    }
}

impl MpResponse {
    /// Quem é o núcleo de boot, no mesmo formato de `MpInfo::hw_id`.
    pub fn bsp_hw_id(&self) -> u64 {
        #[cfg(target_arch = "x86_64")]
        return u64::from(self.bsp_lapic_id);
        #[cfg(not(target_arch = "x86_64"))]
        return self.bsp_mpidr;
    }

    /// # Safety
    /// `cpus` aponta para `cpu_count` ponteiros válidos, como o Limine garante.
    pub unsafe fn cpus(&self) -> &'static [&'static MpInfo] {
        let len = usize::try_from(self.cpu_count).expect("contagem cabe em usize");
        if self.cpus.is_null() || len == 0 {
            return &[];
        }
        // SAFETY: ponteiro e tamanho vêm do bootloader, pelo contrato da função; `&T` e
        // `*const T` têm a mesma representação.
        unsafe { slice::from_raw_parts(self.cpus.cast::<&'static MpInfo>(), len) }
    }
}

#[repr(C)]
pub struct BootloaderInfoResponse {
    pub revision: u64,
    pub name: *const c_char,
    pub version: *const c_char,
}

#[repr(C)]
pub struct ExecutableCmdlineResponse {
    pub revision: u64,
    pub cmdline: *const c_char,
}

#[repr(C)]
pub struct HhdmResponse {
    pub revision: u64,
    pub offset: u64,
}

#[derive(Debug)]
#[repr(C)]
pub struct MemmapEntry {
    pub base: u64,
    pub length: u64,
    pub kind: u64,
}

#[repr(C)]
pub struct MemmapResponse {
    pub revision: u64,
    entry_count: u64,
    entries: *const &'static MemmapEntry,
}

impl MemmapResponse {
    /// # Safety
    /// `entries` aponta para `entry_count` ponteiros válidos, como o Limine garante.
    pub unsafe fn entries(&self) -> &'static [&'static MemmapEntry] {
        let len = usize::try_from(self.entry_count).expect("contagem cabe em usize");
        if self.entries.is_null() || len == 0 {
            return &[];
        }
        // SAFETY: ponteiro e tamanho vêm do bootloader, pelo contrato da função.
        unsafe { slice::from_raw_parts(self.entries, len) }
    }
}

/// Onde o Limine carregou a imagem: base física e base virtual (já com o slide).
#[repr(C)]
pub struct ExecutableAddressResponse {
    pub revision: u64,
    pub physical_base: u64,
    pub virtual_base: u64,
}

/// `struct limine_file`: só `address`, `size` e `string` são lidos; o resto existe pelo tamanho.
#[derive(Debug)]
#[allow(dead_code, reason = "campos do limine.h que o kernel não lê")]
#[repr(C)]
pub struct File {
    pub revision: u64,
    pub address: *const u8,
    pub size: u64,
    pub path: *const c_char,
    pub string: *const c_char,
    media_type: u32,
    unused: u32,
    tftp_ipv4: [u8; 4],
    tftp_port: u32,
    partition_index: u32,
    mbr_disk_id: u32,
    uuids: [[u8; 16]; 3],
}

/// Quantos módulos o kernel lê de uma resposta; o resto é ignorado.
pub const MAX_MODULES: usize = 16;

/// `struct limine_module_response`; o Limine 12.10.0 responde com a revisão 2.
#[repr(C)]
pub struct ModuleResponse {
    pub revision: u64,
    module_count: u64,
    modules: *const *const File,
}

impl ModuleResponse {
    /// Os primeiros `MAX_MODULES` arquivos; ponteiro nulo na lista é pulado.
    /// # Safety
    /// `modules` aponta para `module_count` ponteiros, cada um nulo ou de um `File` válido,
    /// como o Limine garante.
    pub unsafe fn files(&self) -> impl Iterator<Item = &'static File> {
        let len = usize::try_from(self.module_count)
            .unwrap_or(usize::MAX)
            .min(MAX_MODULES);
        let list: &'static [*const File] = if self.modules.is_null() || len == 0 {
            &[]
        } else {
            // SAFETY: ponteiro e tamanho vêm do bootloader, pelo contrato da função.
            unsafe { slice::from_raw_parts(self.modules, len) }
        };
        // SAFETY: cada ponteiro é nulo ou de um `File` válido, pelo contrato da função.
        list.iter().filter_map(|&file| unsafe { file.as_ref() })
    }
}

#[repr(C)]
pub struct PerformanceResponse {
    pub revision: u64,
    pub reset_usec: u64,
    pub init_usec: u64,
    pub exec_usec: u64,
}

/// Campos da revisão 0 da resposta; os das revisões 1 e 2 vêm depois e não são lidos.
#[repr(C)]
pub struct Framebuffer {
    pub address: *mut u8,
    pub width: u64,
    pub height: u64,
    pub pitch: u64,
    pub bpp: u16,
    pub memory_model: u8,
    pub red_mask_size: u8,
    pub red_mask_shift: u8,
    pub green_mask_size: u8,
    pub green_mask_shift: u8,
    pub blue_mask_size: u8,
    pub blue_mask_shift: u8,
    pub(crate) unused: [u8; 7],
    pub edid_size: u64,
    pub edid: *const u8,
}

#[repr(C)]
pub struct FramebufferResponse {
    pub revision: u64,
    framebuffer_count: u64,
    framebuffers: *const &'static Framebuffer,
}

impl FramebufferResponse {
    /// # Safety
    /// `framebuffers` aponta para `framebuffer_count` ponteiros válidos, como o Limine garante.
    pub unsafe fn framebuffers(&self) -> &'static [&'static Framebuffer] {
        let len = usize::try_from(self.framebuffer_count).expect("contagem cabe em usize");
        if self.framebuffers.is_null() || len == 0 {
            return &[];
        }
        // SAFETY: ponteiro e tamanho vêm do bootloader, pelo contrato da função.
        unsafe { slice::from_raw_parts(self.framebuffers, len) }
    }
}

/// Um `MpInfo` com o identificador de hardware dado, para os testes do adaptador.
#[cfg(test)]
pub(crate) fn test_info(hw_id: u64) -> MpInfo {
    #[cfg(target_arch = "x86_64")]
    return MpInfo {
        processor_id: 0,
        lapic_id: u32::try_from(hw_id).unwrap(),
        reserved: 0,
        goto_address: AtomicPtr::new(ptr::null_mut()),
        extra_argument: 0,
    };
    #[cfg(not(target_arch = "x86_64"))]
    return MpInfo {
        processor_id: 0,
        reserved1: 0,
        mpidr: hw_id,
        reserved: 0,
        goto_address: AtomicPtr::new(ptr::null_mut()),
        extra_argument: 0,
    };
}

/// Um `File` com o endereço, o tamanho e a `string` dados, para os testes do adaptador.
#[cfg(test)]
pub(crate) fn test_file(address: u64, size: u64, string: *const c_char) -> File {
    File {
        revision: 0,
        address: ptr::without_provenance(usize::try_from(address).unwrap()),
        size,
        path: ptr::null(),
        string,
        media_type: 0,
        unused: 0,
        tftp_ipv4: [0; 4],
        tftp_port: 0,
        partition_index: 0,
        mbr_disk_id: 0,
        uuids: [[0; 16]; 3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_layout_matches_limine_h() {
        assert_eq!(size_of::<Request<HhdmResponse>>(), 48);
        assert_eq!(core::mem::offset_of!(Request<HhdmResponse>, response), 40);
    }

    #[test]
    fn ids_start_with_common_magic() {
        for id in [
            BOOTLOADER_INFO_ID,
            HHDM_ID,
            MEMMAP_ID,
            PERFORMANCE_ID,
            FRAMEBUFFER_ID,
            EXECUTABLE_ADDRESS_ID,
            MODULE_ID,
            MP_ID,
        ] {
            assert_eq!(id[..2], COMMON_MAGIC);
        }
    }

    #[test]
    fn executable_address_layout_matches_limine_h() {
        assert_eq!(size_of::<ExecutableAddressResponse>(), 24);
        assert_eq!(
            core::mem::offset_of!(ExecutableAddressResponse, physical_base),
            8
        );
        assert_eq!(
            core::mem::offset_of!(ExecutableAddressResponse, virtual_base),
            16
        );
        assert_eq!(size_of::<Request<ExecutableAddressResponse>>(), 48);
        assert_eq!(
            core::mem::offset_of!(Request<ExecutableAddressResponse>, response),
            40
        );
    }

    #[test]
    #[allow(clippy::unreadable_literal, reason = "cópia literal do limine.h")]
    fn executable_address_id_matches_limine_h() {
        assert_eq!(EXECUTABLE_ADDRESS_ID[2], 0x71ba76863cc55f63);
        assert_eq!(EXECUTABLE_ADDRESS_ID[3], 0xb2644a48c516a487);
    }

    #[test]
    fn framebuffer_layout_matches_limine_h() {
        assert_eq!(core::mem::offset_of!(Framebuffer, bpp), 32);
        assert_eq!(core::mem::offset_of!(Framebuffer, memory_model), 34);
        assert_eq!(core::mem::offset_of!(Framebuffer, blue_mask_shift), 40);
        assert_eq!(core::mem::offset_of!(Framebuffer, edid_size), 48);
        assert_eq!(size_of::<Framebuffer>(), 64);
        assert_eq!(size_of::<FramebufferResponse>(), 24);
    }

    #[test]
    #[allow(clippy::unreadable_literal, reason = "cópia literal do limine.h")]
    fn module_layout_and_id_match_limine_h() {
        assert_eq!(MODULE_ID[2], 0x3e7e279702be32af);
        assert_eq!(MODULE_ID[3], 0xca1c4f3bd1280cee);
        assert_eq!(size_of::<File>(), 112);
        assert_eq!(core::mem::offset_of!(File, address), 8);
        assert_eq!(core::mem::offset_of!(File, size), 16);
        assert_eq!(core::mem::offset_of!(File, path), 24);
        assert_eq!(core::mem::offset_of!(File, string), 32);
        assert_eq!(core::mem::offset_of!(File, uuids), 64);
        assert_eq!(size_of::<ModuleResponse>(), 24);
        assert_eq!(core::mem::offset_of!(ModuleResponse, module_count), 8);
        assert_eq!(core::mem::offset_of!(ModuleResponse, modules), 16);
        assert_eq!(size_of::<Request<ModuleResponse>>(), 48);
        assert_eq!(core::mem::offset_of!(Request<ModuleResponse>, response), 40);
    }

    fn response(count: u64, list: &[*const File]) -> ModuleResponse {
        ModuleResponse {
            revision: 2,
            module_count: count,
            modules: if list.is_empty() {
                ptr::null()
            } else {
                list.as_ptr()
            },
        }
    }

    #[test]
    fn module_list_skips_null_entries() {
        let a = test_file(0x1000, 1, ptr::null());
        let b = test_file(0x2000, 2, ptr::null());
        let list = [&raw const a, ptr::null(), &raw const b];
        let module_response = response(3, &list);
        // SAFETY: a lista tem 3 ponteiros, nulos ou de `File` vivos.
        let sizes: Vec<u64> = unsafe { module_response.files() }
            .map(|file| file.size)
            .collect();
        assert_eq!(sizes, [1, 2]);
    }

    #[test]
    fn a_huge_module_count_reads_only_up_to_the_cap() {
        let a = test_file(0x1000, 1, ptr::null());
        let list = [&raw const a; MAX_MODULES];
        let module_response = response(u64::MAX, &list);
        // SAFETY: o teto limita a leitura às `MAX_MODULES` posições de `list`.
        assert_eq!(unsafe { module_response.files() }.count(), MAX_MODULES);
    }

    #[test]
    fn module_list_without_pointer_or_count_is_empty() {
        let a = test_file(0x1000, 1, ptr::null());
        let (none, zero) = (response(5, &[]), response(0, &[&raw const a]));
        // SAFETY: lista nula não é lida.
        assert_eq!(unsafe { none.files() }.count(), 0);
        // SAFETY: contagem zero não lê a lista.
        assert_eq!(unsafe { zero.files() }.count(), 0);
    }

    #[test]
    fn mp_request_layout_matches_limine_h() {
        assert_eq!(size_of::<MpRequest>(), 56);
        assert_eq!(core::mem::offset_of!(MpRequest, response), 40);
        assert_eq!(core::mem::offset_of!(MpRequest, flags), 48);
    }

    #[test]
    #[allow(clippy::unreadable_literal, reason = "cópia literal do limine.h")]
    fn mp_id_matches_limine_h() {
        assert_eq!(MP_ID[2], 0x95a67b819a1b857e);
        assert_eq!(MP_ID[3], 0xa0b61b723b6a73e0);
        assert_eq!(MP_REQUEST_X2APIC, 1 << 0);
    }

    #[test]
    fn mp_x86_layout_matches_limine_h() {
        assert_eq!(size_of::<MpInfoX86>(), 32);
        assert_eq!(core::mem::offset_of!(MpInfoX86, lapic_id), 4);
        assert_eq!(core::mem::offset_of!(MpInfoX86, goto_address), 16);
        assert_eq!(core::mem::offset_of!(MpInfoX86, extra_argument), 24);
        assert_eq!(size_of::<MpResponseX86>(), 32);
        assert_eq!(core::mem::offset_of!(MpResponseX86, flags), 8);
        assert_eq!(core::mem::offset_of!(MpResponseX86, bsp_lapic_id), 12);
        assert_eq!(core::mem::offset_of!(MpResponseX86, cpu_count), 16);
        assert_eq!(core::mem::offset_of!(MpResponseX86, cpus), 24);
    }

    #[test]
    fn mp_aarch64_layout_matches_limine_h() {
        assert_eq!(size_of::<MpInfoAarch64>(), 40);
        assert_eq!(core::mem::offset_of!(MpInfoAarch64, mpidr), 8);
        assert_eq!(core::mem::offset_of!(MpInfoAarch64, goto_address), 24);
        assert_eq!(core::mem::offset_of!(MpInfoAarch64, extra_argument), 32);
        assert_eq!(size_of::<MpResponseAarch64>(), 40);
        assert_eq!(core::mem::offset_of!(MpResponseAarch64, bsp_mpidr), 16);
        assert_eq!(core::mem::offset_of!(MpResponseAarch64, cpu_count), 24);
        assert_eq!(core::mem::offset_of!(MpResponseAarch64, cpus), 32);
    }

    #[test]
    fn mp_info_reports_hardware_id_and_argument() {
        let mut info = test_info(7);
        info.extra_argument = 42;
        assert_eq!((info.hw_id(), info.argument()), (7, 42));
    }

    #[test]
    fn base_revision_is_unsupported_until_bootloader_clears_it() {
        assert!(!BaseRevision::new(BASE_REVISION).is_supported());
        assert!(BaseRevision::new(0).is_supported());
    }

    #[test]
    fn unanswered_request_has_no_response() {
        let request = Request::<HhdmResponse>::new(HHDM_ID);
        // SAFETY: nenhuma resposta foi escrita; o ponteiro é nulo.
        assert!(unsafe { request.response() }.is_none());
    }
}
