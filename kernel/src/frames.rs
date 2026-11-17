//! O alocador de frames do kernel, compartilhado entre `Memory` e o heap.
//! Ordem de lock: heap, depois frames. Quem segura `FRAMES` nunca chama o heap.
//! Sem cache por CPU: DEBT-0018.

use rustos_hal::{FrameSource, PhysAddr};
use rustos_mm::{BuddyAllocator, FrameAllocator, FrameError, PhysFrame};

use crate::sync::IrqLock;

static FRAMES: IrqLock<Option<BuddyAllocator<'static>>> = IrqLock::new(None);

/// Cada chamada trava `FRAMES` só pelo tempo dela; o lock nunca fica preso entre duas.
/// Só `install` cria um, então o alocador existe sempre que há um `Frames`.
#[derive(Clone, Copy, Debug)]
pub struct Frames {
    _installed: (),
}

/// Entrega o alocador ao kernel. Uma vez só.
pub fn install(allocator: BuddyAllocator<'static>) -> Frames {
    let mut slot = FRAMES.lock();
    assert!(slot.is_none(), "frames: alocador instalado duas vezes");
    *slot = Some(allocator);
    Frames { _installed: () }
}

#[allow(
    clippy::unused_self,
    reason = "`self` é a prova de que `install` rodou; o estado é global"
)]
impl Frames {
    fn with<R>(run: impl FnOnce(&mut BuddyAllocator<'static>) -> R) -> R {
        let mut slot = FRAMES.lock();
        match slot.as_mut() {
            Some(allocator) => run(allocator),
            None => unreachable!("Frames só existe depois de install"),
        }
    }

    pub fn free_frames(self) -> usize {
        Self::with(|allocator| allocator.free_frames())
    }

    pub fn total_frames(self) -> usize {
        Self::with(|allocator| allocator.total_frames())
    }

    pub fn allocate(self) -> Option<PhysFrame> {
        Self::with(BuddyAllocator::allocate)
    }

    pub fn deallocate(self, frame: PhysFrame) -> Result<(), FrameError> {
        Self::with(|allocator| allocator.deallocate(frame))
    }
}

impl FrameSource for Frames {
    fn allocate_frame(&mut self) -> Option<PhysAddr> {
        self.allocate().map(PhysFrame::start)
    }
}

impl FrameAllocator for Frames {
    fn allocate_contiguous(&mut self, count: usize) -> Option<PhysFrame> {
        Self::with(|allocator| allocator.allocate_contiguous(count))
    }

    fn deallocate_contiguous(&mut self, first: PhysFrame, count: usize) -> Result<(), FrameError> {
        Self::with(|allocator| allocator.deallocate_contiguous(first, count))
    }
}
