//! Leitura do mapa de memória: totais e onde cabe uma estrutura do kernel.

use rustos_hal::{MemoryRegion, PhysAddr, RegionKind, addr::align_up};

use crate::FRAME_SIZE;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemorySummary {
    pub regions: usize,
    pub usable_bytes: u64,
    /// Memória do bootloader que o kernel devolve ao alocador depois de copiar o que lê.
    pub reclaimable_bytes: u64,
    pub other_bytes: u64,
    /// Do início da primeira região utilizável ou recuperável ao fim da última: o que
    /// o buddy cobre, para poder receber a memória do bootloader depois.
    pub managed_span: Option<(PhysAddr, PhysAddr)>,
}

impl MemorySummary {
    pub fn span_frames(&self) -> u64 {
        self.managed_span.map_or(0, |(start, end)| {
            (end.as_u64() - start.as_u64()) / FRAME_SIZE
        })
    }
}

pub fn summarize(regions: impl IntoIterator<Item = MemoryRegion>) -> MemorySummary {
    let mut summary = MemorySummary::default();
    for region in regions {
        summary.regions += 1;
        let total = match region.kind {
            RegionKind::Usable => &mut summary.usable_bytes,
            RegionKind::BootloaderReclaimable => &mut summary.reclaimable_bytes,
            _ => {
                summary.other_bytes = summary.other_bytes.saturating_add(region.len);
                continue;
            }
        };
        *total = total.saturating_add(region.len);
        let start = region.start.align_down(FRAME_SIZE);
        let end = PhysAddr::new(region.end());
        summary.managed_span = Some(match summary.managed_span {
            None => (start, end),
            Some((low, high)) => (low.min(start), high.max(end)),
        });
    }
    summary
}

/// Primeira região utilizável com `bytes` livres a partir de um início alinhado ao frame.
pub fn find_free_block(
    regions: impl IntoIterator<Item = MemoryRegion>,
    bytes: u64,
) -> Option<PhysAddr> {
    regions
        .into_iter()
        .filter(MemoryRegion::is_usable)
        .find_map(|region| {
            let start = align_up(region.start.as_u64(), FRAME_SIZE)?;
            let end = start.checked_add(bytes)?;
            (end <= region.end()).then_some(PhysAddr::new(start))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> [MemoryRegion; 4] {
        [
            MemoryRegion::new(0x0, 0x1000, RegionKind::Reserved),
            MemoryRegion::new(0x1000, 0x2000, RegionKind::Usable),
            MemoryRegion::new(0x3000, 0x1000, RegionKind::KernelAndModules),
            MemoryRegion::new(0x10_0000, 0x10_0000, RegionKind::Usable),
        ]
    }

    #[test]
    fn summary_counts_usable_and_other_bytes() {
        let summary = summarize(map());
        assert_eq!(summary.regions, 4);
        assert_eq!(summary.usable_bytes, 0x10_2000);
        assert_eq!(summary.other_bytes, 0x2000);
    }

    #[test]
    fn summary_span_goes_from_first_to_last_usable() {
        let summary = summarize(map());
        assert_eq!(
            summary.managed_span,
            Some((PhysAddr::new(0x1000), PhysAddr::new(0x20_0000)))
        );
        assert_eq!(summary.span_frames(), 0x1ff);
    }

    #[test]
    fn reclaimable_regions_count_apart_and_extend_the_span() {
        let regions = [
            MemoryRegion::new(0x1000, 0x1000, RegionKind::Usable),
            MemoryRegion::new(0x4000, 0x2000, RegionKind::BootloaderReclaimable),
            MemoryRegion::new(0x8000, 0x1000, RegionKind::Reserved),
        ];
        let summary = summarize(regions);
        assert_eq!(summary.usable_bytes, 0x1000);
        assert_eq!(summary.reclaimable_bytes, 0x2000);
        assert_eq!(summary.other_bytes, 0x1000);
        assert_eq!(
            summary.managed_span,
            Some((PhysAddr::new(0x1000), PhysAddr::new(0x6000)))
        );
    }

    #[test]
    fn empty_map_has_no_span() {
        let summary = summarize([]);
        assert_eq!(summary.managed_span, None);
        assert_eq!(summary.span_frames(), 0);
    }

    #[test]
    fn free_block_skips_regions_too_small() {
        assert_eq!(find_free_block(map(), 0x2000), Some(PhysAddr::new(0x1000)));
        assert_eq!(
            find_free_block(map(), 0x3000),
            Some(PhysAddr::new(0x10_0000))
        );
    }

    #[test]
    fn free_block_respects_frame_alignment() {
        let regions = [MemoryRegion::new(0x1800, 0x1800, RegionKind::Usable)];
        assert_eq!(
            find_free_block(regions, 0x1000),
            Some(PhysAddr::new(0x2000))
        );
        assert_eq!(find_free_block(regions, 0x1001), None);
    }

    #[test]
    fn free_block_ignores_non_usable_regions() {
        let regions = [MemoryRegion::new(
            0x0,
            0x10_0000,
            RegionKind::BootloaderReclaimable,
        )];
        assert_eq!(find_free_block(regions, 0x1000), None);
    }
}
