//! Mapa de memória física como o núcleo o enxerga, independente do bootloader.

use crate::PhysAddr;

/// O padrão com que o kernel pinta as pilhas das threads: o pico é até onde ele sumiu, e a
/// limpeza da pilha na volta ao usuário (SDD 17) repinta com ele o que a chamada usou. Nenhum
/// valor que uma pilha escreva de propósito.
pub const STACK_PAINT: u64 = 0x5253_5441_434b_2121;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    Usable,
    Reserved,
    AcpiReclaimable,
    AcpiNvs,
    BadMemory,
    BootloaderReclaimable,
    KernelAndModules,
    Framebuffer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryRegion {
    pub start: PhysAddr,
    pub len: u64,
    pub kind: RegionKind,
}

impl MemoryRegion {
    pub const fn new(start: u64, len: u64, kind: RegionKind) -> Self {
        Self {
            start: PhysAddr::new(start),
            len,
            kind,
        }
    }

    /// Fim exclusivo; satura em `u64::MAX` em vez de transbordar.
    pub const fn end(&self) -> u64 {
        self.start.as_u64().saturating_add(self.len)
    }

    pub const fn is_usable(&self) -> bool {
        matches!(self.kind, RegionKind::Usable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_is_exclusive() {
        let region = MemoryRegion::new(0x1000, 0x2000, RegionKind::Usable);
        assert_eq!(region.end(), 0x3000);
    }

    #[test]
    fn end_saturates() {
        let region = MemoryRegion::new(u64::MAX - 1, 10, RegionKind::Reserved);
        assert_eq!(region.end(), u64::MAX);
    }

    #[test]
    fn only_usable_is_usable() {
        assert!(MemoryRegion::new(0, 1, RegionKind::Usable).is_usable());
        assert!(!MemoryRegion::new(0, 1, RegionKind::BootloaderReclaimable).is_usable());
    }
}
