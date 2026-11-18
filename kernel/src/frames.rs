//! O alocador de frames do kernel, compartilhado entre `Memory` e o heap, com o cache por CPU
//! na frente do buddy para o frame solto. Ordem de lock: heap, cache da CPU, frames. Quem
//! segura `FRAMES` nunca chama o heap nem o cache.

use core::sync::atomic::{AtomicBool, Ordering};

use rustos_hal::{FrameSource, MAX_CPUS, PhysAddr};
use rustos_mm::{BuddyAllocator, FrameAllocator, FrameCache, FrameError, PhysFrame};

use crate::percpu;
use crate::sync::IrqLock;

/// Frames que cada CPU guarda: a metade vai e vem do buddy de uma vez (`FrameCache::BATCH`).
pub const CACHE_CAP: usize = 32;
pub type Cache = FrameCache<CACHE_CAP>;

static FRAMES: IrqLock<Option<BuddyAllocator<'static>>> = IrqLock::new(None);
/// O cache por CPU está ligado: frame solto passa por ele em vez do lock do buddy.
static CACHE_ON: AtomicBool = AtomicBool::new(true);

/// Liga ou desliga o cache por CPU. Desligar não devolve o que ele guarda: `flush_caches`.
#[cfg(feature = "selftest")]
pub fn set_cache(on: bool) {
    CACHE_ON.store(on, Ordering::Release);
}

/// Devolve um frame que `allocate_frame` entregou.
#[cfg(feature = "selftest")]
pub fn free_frame(frames: Frames, frame: PhysAddr) {
    frames.deallocate(PhysFrame::containing(frame)).ok();
}

/// O handle, se o alocador já foi instalado.
#[cfg(feature = "selftest")]
pub fn handle() -> Option<Frames> {
    FRAMES.lock().is_some().then_some(Frames { _installed: () })
}

/// Disputas pelo lock do buddy desde o boot.
pub fn contentions() -> u32 {
    FRAMES.contentions()
}

/// O buddy por trás do cache: cada chamada trava `FRAMES` só pela duração dela.
struct Backing;

impl FrameAllocator for Backing {
    fn allocate_contiguous(&mut self, count: usize) -> Option<PhysFrame> {
        Frames::with(|allocator| allocator.allocate_contiguous(count))
    }

    fn deallocate_contiguous(&mut self, first: PhysFrame, count: usize) -> Result<(), FrameError> {
        Frames::with(|allocator| allocator.deallocate_contiguous(first, count))
    }

    fn verify_used(&self, first: PhysFrame, count: usize) -> Result<(), FrameError> {
        Frames::with(|allocator| allocator.verify_used(first, count))
    }
}

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

    /// Livres no buddy mais os que os caches das CPUs guardam.
    pub fn free_frames(self) -> usize {
        let cached: usize = (0..MAX_CPUS)
            .map(|index| percpu::get(index).frames.lock().len())
            .sum();
        Self::with(|allocator| allocator.free_frames()) + cached
    }

    #[cfg(feature = "selftest")]
    pub fn total_frames(self) -> usize {
        Self::with(|allocator| allocator.total_frames())
    }

    pub fn allocate(self) -> Option<PhysFrame> {
        if CACHE_ON.load(Ordering::Acquire) {
            return percpu::current().frames.lock().allocate(&mut Backing);
        }
        Self::with(BuddyAllocator::allocate)
    }

    /// Entrega ao alocador a memória que o kernel já não usa (a do bootloader).
    pub fn release_range(self, start: PhysAddr, len: u64) {
        Self::with(|allocator| allocator.release_range(start, len));
    }

    pub fn deallocate(self, frame: PhysFrame) -> Result<(), FrameError> {
        if CACHE_ON.load(Ordering::Acquire) {
            return percpu::current()
                .frames
                .lock()
                .deallocate(frame, &mut Backing);
        }
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
        if count == 1 {
            return self.allocate();
        }
        Self::with(|allocator| allocator.allocate_contiguous(count))
    }

    fn deallocate_contiguous(&mut self, first: PhysFrame, count: usize) -> Result<(), FrameError> {
        if count == 1 {
            return self.deallocate(first);
        }
        Self::with(|allocator| allocator.deallocate_contiguous(first, count))
    }

    /// Um frame só não é conferido com o cache ligado: conferir custaria o lock que o cache
    /// evita (DEBT-0017).
    fn verify_used(&self, first: PhysFrame, count: usize) -> Result<(), FrameError> {
        if count == 1 && CACHE_ON.load(Ordering::Acquire) {
            return Ok(());
        }
        Self::with(|allocator| allocator.verify_used(first, count))
    }
}
