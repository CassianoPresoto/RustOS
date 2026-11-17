//! Estruturas do protocolo Limine, campo a campo como em `limine.h` (limine-protocol).

use core::cell::UnsafeCell;
use core::ffi::c_char;
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
pub const MEMMAP_ID: [u64; 4] = id(0x67cf_3d9d_378a_806f, 0xe304_acdf_c50c_3c62);
pub const PERFORMANCE_ID: [u64; 4] = id(0x6b50_ad9b_f36d_13ad, 0xdc4c_7e88_fc75_9e17);
pub const FRAMEBUFFER_ID: [u64; 4] = id(0x9d58_27dc_d881_dd75, 0xa314_8604_f6fa_b11b);
pub const EXECUTABLE_ADDRESS_ID: [u64; 4] = id(0x71ba_7686_3cc5_5f63, 0xb264_4a48_c516_a487);

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

#[repr(C)]
pub struct BootloaderInfoResponse {
    pub revision: u64,
    pub name: *const c_char,
    pub version: *const c_char,
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
