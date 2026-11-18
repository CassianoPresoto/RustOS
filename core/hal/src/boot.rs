//! Porta de entrada: o que o núcleo precisa saber do bootloader.

use crate::{FramebufferInfo, MemoryRegion, PhysAddr, VirtAddr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootloaderId<'a> {
    pub name: &'a str,
    pub version: &'a str,
}

/// Marcos do boot em microssegundos, numa origem arbitrária e comum aos três.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootTimeline {
    pub reset_us: u64,
    pub loader_start_us: u64,
    pub kernel_handoff_us: u64,
}

impl BootTimeline {
    /// Firmware: do reset até o bootloader começar.
    pub const fn firmware_us(&self) -> u64 {
        self.loader_start_us.saturating_sub(self.reset_us)
    }

    /// Bootloader: do início dele até entregar o controle ao kernel.
    pub const fn loader_us(&self) -> u64 {
        self.kernel_handoff_us.saturating_sub(self.loader_start_us)
    }
}

/// Onde a imagem do kernel foi carregada: o físico e o virtual do início dela.
/// O mapeamento é de deslocamento único (a imagem é contígua em memória física).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernelImage {
    pub physical_base: PhysAddr,
    pub virtual_base: VirtAddr,
}

impl KernelImage {
    /// Endereço físico de um endereço virtual da imagem; `None` abaixo da base.
    pub const fn physical_of(&self, virt: VirtAddr) -> Option<PhysAddr> {
        match virt.as_u64().checked_sub(self.virtual_base.as_u64()) {
            Some(delta) => self.physical_base.checked_add(delta),
            None => None,
        }
    }
}

/// Um núcleo que o bootloader entregou, o de boot incluído.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuDescriptor {
    /// LAPIC ID no x86_64; afinidade do `MPIDR_EL1` (Aff3..Aff0 sem os bits de controle) no aarch64.
    pub hw_id: u64,
    pub is_boot: bool,
    /// Identificador opaco do núcleo no bootloader, para soltá-lo.
    pub token: usize,
}

pub trait BootInfo {
    fn memory_regions(&self) -> impl Iterator<Item = MemoryRegion> + '_;

    /// Deslocamento do mapa direto da memória física: `virtual = físico + offset`.
    fn direct_map_offset(&self) -> u64;

    fn bootloader(&self) -> Option<BootloaderId<'_>>;

    fn timeline(&self) -> Option<BootTimeline>;

    /// O primeiro framebuffer de 32 bpp em RGB, se o firmware entregou um.
    fn framebuffer(&self) -> Option<FramebufferInfo>;

    /// Base física e virtual da imagem do kernel.
    fn kernel_image(&self) -> Option<KernelImage>;

    /// Os núcleos do bootloader; vazio se ele não respondeu ao pedido de MP.
    fn cpus(&self) -> impl Iterator<Item = CpuDescriptor> + '_;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_splits_firmware_and_loader() {
        let timeline = BootTimeline {
            reset_us: 100,
            loader_start_us: 400,
            kernel_handoff_us: 450,
        };
        assert_eq!(timeline.firmware_us(), 300);
        assert_eq!(timeline.loader_us(), 50);
    }

    #[test]
    fn kernel_image_translates_with_one_offset() {
        let image = KernelImage {
            physical_base: PhysAddr::new(0x20_0000),
            virtual_base: VirtAddr::new(0xffff_ffff_8000_0000),
        };
        let phys = image.physical_of(VirtAddr::new(0xffff_ffff_8000_3000));
        assert_eq!(phys, Some(PhysAddr::new(0x20_3000)));
        assert_eq!(
            image.physical_of(VirtAddr::new(0xffff_ffff_7fff_f000)),
            None
        );
    }

    #[test]
    fn timeline_never_underflows() {
        let timeline = BootTimeline {
            reset_us: 500,
            loader_start_us: 0,
            kernel_handoff_us: 0,
        };
        assert_eq!(timeline.firmware_us(), 0);
    }
}
