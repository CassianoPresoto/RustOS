//! Os pedidos que o kernel faz ao Limine, entre os marcadores de início e fim.
//! O linker script mantém as três seções em ordem e com `KEEP`; no host não há seção.

use crate::protocol::{
    BASE_REVISION, BOOTLOADER_INFO_ID, BaseRevision, BootloaderInfoResponse, EXECUTABLE_ADDRESS_ID,
    ExecutableAddressResponse, FRAMEBUFFER_ID, FramebufferResponse, HHDM_ID, HhdmResponse,
    MEMMAP_ID, Marker, MemmapResponse, PERFORMANCE_ID, PerformanceResponse, REQUESTS_END_MARKER,
    REQUESTS_START_MARKER, Request,
};

#[used]
#[cfg_attr(target_os = "none", unsafe(link_section = ".requests_start_marker"))]
static START_MARKER: Marker<4> = Marker::new(REQUESTS_START_MARKER);

#[used]
#[cfg_attr(target_os = "none", unsafe(link_section = ".requests"))]
static REVISION: BaseRevision = BaseRevision::new(BASE_REVISION);

#[used]
#[cfg_attr(target_os = "none", unsafe(link_section = ".requests"))]
static BOOTLOADER_INFO: Request<BootloaderInfoResponse> = Request::new(BOOTLOADER_INFO_ID);

#[used]
#[cfg_attr(target_os = "none", unsafe(link_section = ".requests"))]
static HHDM: Request<HhdmResponse> = Request::new(HHDM_ID);

#[used]
#[cfg_attr(target_os = "none", unsafe(link_section = ".requests"))]
static MEMMAP: Request<MemmapResponse> = Request::new(MEMMAP_ID);

#[used]
#[cfg_attr(target_os = "none", unsafe(link_section = ".requests"))]
static PERFORMANCE: Request<PerformanceResponse> = Request::new(PERFORMANCE_ID);

#[used]
#[cfg_attr(target_os = "none", unsafe(link_section = ".requests"))]
static FRAMEBUFFER: Request<FramebufferResponse> = Request::new(FRAMEBUFFER_ID);

#[used]
#[cfg_attr(target_os = "none", unsafe(link_section = ".requests"))]
static EXECUTABLE_ADDRESS: Request<ExecutableAddressResponse> = Request::new(EXECUTABLE_ADDRESS_ID);

#[used]
#[cfg_attr(target_os = "none", unsafe(link_section = ".requests_end_marker"))]
static END_MARKER: Marker<2> = Marker::new(REQUESTS_END_MARKER);

pub fn base_revision_supported() -> bool {
    REVISION.is_supported()
}

/// # Safety
/// Ver `Request::response`.
pub unsafe fn bootloader_info() -> Option<&'static BootloaderInfoResponse> {
    // SAFETY: repassada a quem chama.
    unsafe { BOOTLOADER_INFO.response() }
}

/// # Safety
/// Ver `Request::response`.
pub unsafe fn hhdm() -> Option<&'static HhdmResponse> {
    // SAFETY: repassada a quem chama.
    unsafe { HHDM.response() }
}

/// # Safety
/// Ver `Request::response`.
pub unsafe fn memmap() -> Option<&'static MemmapResponse> {
    // SAFETY: repassada a quem chama.
    unsafe { MEMMAP.response() }
}

/// # Safety
/// Ver `Request::response`.
pub unsafe fn performance() -> Option<&'static PerformanceResponse> {
    // SAFETY: repassada a quem chama.
    unsafe { PERFORMANCE.response() }
}

/// # Safety
/// Ver `Request::response`.
pub unsafe fn executable_address() -> Option<&'static ExecutableAddressResponse> {
    // SAFETY: repassada a quem chama.
    unsafe { EXECUTABLE_ADDRESS.response() }
}

/// # Safety
/// Ver `Request::response`.
pub unsafe fn framebuffer() -> Option<&'static FramebufferResponse> {
    // SAFETY: repassada a quem chama.
    unsafe { FRAMEBUFFER.response() }
}
