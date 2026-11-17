//! Adaptador do protocolo de boot Limine (revisão base 6): traduz as respostas
//! do bootloader para `hal::BootInfo`. Protocolo escrito aqui, sem crate externo.

#![cfg_attr(not(test), no_std)]

mod protocol;
mod requests;

use core::ffi::{CStr, c_char};

use rustos_hal::{BootInfo, BootTimeline, BootloaderId, MemoryRegion, PhysAddr, RegionKind};

pub use protocol::BASE_REVISION;
use protocol::MemmapEntry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootError {
    UnsupportedBaseRevision,
    MissingMemoryMap,
    MissingDirectMap,
}

/// Visão do kernel sobre as respostas do Limine. Só existe se o essencial chegou.
#[derive(Debug)]
pub struct LimineBoot {
    entries: &'static [&'static MemmapEntry],
    direct_map_offset: u64,
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
        // SAFETY: o Limine garante `entry_count` ponteiros válidos em `entries`.
        let entries = unsafe { memmap.entries() };
        Ok(Self {
            entries,
            direct_map_offset: hhdm.offset,
        })
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

    #[test]
    fn null_c_string_is_none() {
        // SAFETY: ponteiro nulo é aceito pelo contrato.
        assert_eq!(unsafe { c_str(core::ptr::null()) }, None);
    }

    #[test]
    fn c_string_is_read_until_nul() {
        let raw = c"Limine";
        // SAFETY: literal C terminado em zero e estático.
        assert_eq!(unsafe { c_str(raw.as_ptr()) }, Some("Limine"));
    }
}
