//! Porta de paginação: mapear página virtual em frame físico.

use crate::{PhysAddr, VirtAddr};

/// Granularidade de página e de frame em todas as ISAs suportadas hoje.
pub const PAGE_SIZE: u64 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheMode {
    WriteBack,
    /// Sem cache e sem reordenação: registradores de dispositivo.
    Device,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageFlags {
    pub writable: bool,
    pub executable: bool,
    pub cache: CacheMode,
}

impl PageFlags {
    pub const DEVICE: Self = Self {
        writable: true,
        executable: false,
        cache: CacheMode::Device,
    };
    pub const DATA: Self = Self {
        writable: true,
        executable: false,
        cache: CacheMode::WriteBack,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    Misaligned,
    AlreadyMapped(VirtAddr),
    /// Uma página grande já cobre o endereço; não se desce mais na tabela.
    HugePageInTheWay(VirtAddr),
    OutOfFrames,
    OutOfAddressSpace,
}

/// Fonte de frames físicos para as tabelas de página intermediárias.
pub trait FrameSource {
    fn allocate_frame(&mut self) -> Option<PhysAddr>;
}

pub trait PageMapper {
    fn map_page<F: FrameSource>(
        &mut self,
        virt: VirtAddr,
        phys: PhysAddr,
        flags: PageFlags,
        frames: &mut F,
    ) -> Result<(), MapError>;
}
