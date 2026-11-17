//! O que o boot entregou, copiado para memória do kernel. Depois da cópia nada
//! lê as respostas do bootloader, e a memória delas pode voltar ao alocador.

use alloc::string::String;
use alloc::vec::Vec;

use rustos_hal::{
    BootInfo, BootTimeline, BootloaderId, FramebufferInfo, KernelImage, MemoryRegion, RegionKind,
};

#[derive(Clone, Debug)]
pub struct BootRecord {
    regions: Vec<MemoryRegion>,
    direct_map_offset: u64,
    bootloader: Option<(String, String)>,
    timeline: Option<BootTimeline>,
    framebuffer: Option<FramebufferInfo>,
    kernel_image: Option<KernelImage>,
}

impl BootRecord {
    /// Copia tudo o que o kernel ainda lê de `source`. Precisa do heap.
    pub fn capture(source: &impl BootInfo) -> Self {
        Self {
            regions: source.memory_regions().collect(),
            direct_map_offset: source.direct_map_offset(),
            bootloader: source
                .bootloader()
                .map(|id| (String::from(id.name), String::from(id.version))),
            timeline: source.timeline(),
            framebuffer: source.framebuffer(),
            kernel_image: source.kernel_image(),
        }
    }

    /// Passa as regiões `BootloaderReclaimable` a `Usable`, depois de o alocador
    /// recebê-las. Devolve quantos bytes mudaram de tipo.
    pub fn mark_reclaimed(&mut self) -> u64 {
        let mut bytes = 0;
        for region in &mut self.regions {
            if region.kind == RegionKind::BootloaderReclaimable {
                region.kind = RegionKind::Usable;
                bytes += region.len;
            }
        }
        bytes
    }
}

impl BootInfo for BootRecord {
    fn memory_regions(&self) -> impl Iterator<Item = MemoryRegion> + '_ {
        self.regions.iter().copied()
    }

    fn direct_map_offset(&self) -> u64 {
        self.direct_map_offset
    }

    fn bootloader(&self) -> Option<BootloaderId<'_>> {
        self.bootloader
            .as_ref()
            .map(|(name, version)| BootloaderId { name, version })
    }

    fn timeline(&self) -> Option<BootTimeline> {
        self.timeline
    }

    fn framebuffer(&self) -> Option<FramebufferInfo> {
        self.framebuffer
    }

    fn kernel_image(&self) -> Option<KernelImage> {
        self.kernel_image
    }
}
