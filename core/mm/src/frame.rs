//! Frame físico de 4 KiB e os erros dos alocadores de frames.

use rustos_hal::{PAGE_SIZE, PhysAddr};

pub const FRAME_SIZE: u64 = PAGE_SIZE;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysFrame(PhysAddr);

impl PhysFrame {
    pub const fn containing(addr: PhysAddr) -> Self {
        Self(addr.align_down(FRAME_SIZE))
    }

    pub const fn start(self) -> PhysAddr {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    StorageTooSmall {
        needed_words: usize,
        given_words: usize,
    },
    MisalignedBase(PhysAddr),
    OutOfRange(PhysAddr),
    DoubleFree(PhysAddr),
    /// Bloco que não começa num múltiplo do próprio tamanho.
    MisalignedBlock(PhysAddr),
    OrderTooLarge(usize),
    /// Mais frames do que os níveis do bitmap hierárquico alcançam.
    TooManyFrames(usize),
}
