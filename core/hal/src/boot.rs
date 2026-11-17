//! Porta de entrada: o que o núcleo precisa saber do bootloader.

use crate::MemoryRegion;

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

pub trait BootInfo {
    fn memory_regions(&self) -> impl Iterator<Item = MemoryRegion> + '_;

    /// Deslocamento do mapa direto da memória física: `virtual = físico + offset`.
    fn direct_map_offset(&self) -> u64;

    fn bootloader(&self) -> Option<BootloaderId<'_>>;

    fn timeline(&self) -> Option<BootTimeline>;
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
    fn timeline_never_underflows() {
        let timeline = BootTimeline {
            reset_us: 500,
            loader_start_us: 0,
            kernel_handoff_us: 0,
        };
        assert_eq!(timeline.firmware_us(), 0);
    }
}
