//! Cache de frames por CPU (magazine): pilha de frames soltos que o buddy não vê.
//! O núcleo não trava nada: quem chama garante que só uma CPU usa cada cache.

use rustos_hal::{FrameSource, PhysAddr};

use crate::buddy::FrameAllocator;
use crate::frame::{FrameError, PhysFrame};

/// Pilha de até `CAP` frames de ordem 0. Cheia, devolve a metade de cima ao buddy;
/// vazia, pede a metade de baixo dele.
#[derive(Debug)]
pub struct FrameCache<const CAP: usize> {
    frames: [PhysAddr; CAP],
    len: usize,
}

impl<const CAP: usize> FrameCache<CAP> {
    const BATCH: usize = {
        assert!(CAP >= 2);
        CAP / 2
    };

    pub const fn new() -> Self {
        Self {
            frames: [PhysAddr::new(0); CAP],
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn allocate<A: FrameAllocator>(&mut self, backing: &mut A) -> Option<PhysFrame> {
        if self.len == 0 {
            self.refill(backing);
        }
        self.len = self.len.checked_sub(1)?;
        Some(PhysFrame::containing(self.frames[self.len]))
    }

    pub fn deallocate<A: FrameAllocator>(
        &mut self,
        frame: PhysFrame,
        backing: &mut A,
    ) -> Result<(), FrameError> {
        if self.len == CAP {
            self.drain(Self::BATCH, backing)?;
        }
        self.frames[self.len] = frame.start();
        self.len += 1;
        Ok(())
    }

    /// Devolve todos os frames ao buddy, como antes de tirar a CPU do ar.
    pub fn flush<A: FrameAllocator>(&mut self, backing: &mut A) -> Result<(), FrameError> {
        self.drain(self.len, backing)
    }

    /// Vista que serve `FrameAllocator` e `FrameSource` passando pelo cache.
    pub fn through<'a, A: FrameAllocator>(&'a mut self, backing: &'a mut A) -> Cached<'a, CAP, A> {
        Cached {
            cache: self,
            backing,
        }
    }

    fn refill<A: FrameAllocator>(&mut self, backing: &mut A) {
        while self.len < Self::BATCH {
            let Some(frame) = backing.allocate_contiguous(1) else {
                return;
            };
            self.frames[self.len] = frame.start();
            self.len += 1;
        }
    }

    fn drain<A: FrameAllocator>(
        &mut self,
        count: usize,
        backing: &mut A,
    ) -> Result<(), FrameError> {
        for _ in 0..count.min(self.len) {
            self.len -= 1;
            backing.deallocate_contiguous(PhysFrame::containing(self.frames[self.len]), 1)?;
        }
        Ok(())
    }
}

impl<const CAP: usize> Default for FrameCache<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

/// Um cache por CPU, com `CPUS` fixo em compilação.
#[derive(Debug)]
pub struct FrameCaches<const CPUS: usize, const CAP: usize> {
    caches: [FrameCache<CAP>; CPUS],
}

impl<const CPUS: usize, const CAP: usize> FrameCaches<CPUS, CAP> {
    pub const fn new() -> Self {
        Self {
            caches: [const { FrameCache::new() }; CPUS],
        }
    }

    pub fn cpu(&mut self, id: usize) -> Option<&mut FrameCache<CAP>> {
        self.caches.get_mut(id)
    }

    /// Para o kernel entregar cada cache à sua CPU.
    pub fn as_mut_slice(&mut self) -> &mut [FrameCache<CAP>] {
        &mut self.caches
    }
}

impl<const CPUS: usize, const CAP: usize> Default for FrameCaches<CPUS, CAP> {
    fn default() -> Self {
        Self::new()
    }
}

/// Frame solto passa pelo cache; corrida de mais de um frame vai direto ao buddy.
#[derive(Debug)]
pub struct Cached<'a, const CAP: usize, A> {
    cache: &'a mut FrameCache<CAP>,
    backing: &'a mut A,
}

impl<const CAP: usize, A: FrameAllocator> FrameAllocator for Cached<'_, CAP, A> {
    fn allocate_contiguous(&mut self, count: usize) -> Option<PhysFrame> {
        if count == 1 {
            self.cache.allocate(self.backing)
        } else {
            self.backing.allocate_contiguous(count)
        }
    }

    fn deallocate_contiguous(&mut self, first: PhysFrame, count: usize) -> Result<(), FrameError> {
        if count == 1 {
            self.cache.deallocate(first, self.backing)
        } else {
            self.backing.deallocate_contiguous(first, count)
        }
    }
}

impl<const CAP: usize, A: FrameAllocator> FrameSource for Cached<'_, CAP, A> {
    fn allocate_frame(&mut self) -> Option<PhysAddr> {
        self.cache.allocate(self.backing).map(PhysFrame::start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::BuddyAllocator;
    use crate::frame::FRAME_SIZE;

    /// Conta quantas vezes o buddy é tocado.
    struct Counting<'a, 'b> {
        buddy: &'b mut BuddyAllocator<'a>,
        calls: usize,
    }

    impl FrameAllocator for Counting<'_, '_> {
        fn allocate_contiguous(&mut self, count: usize) -> Option<PhysFrame> {
            self.calls += 1;
            self.buddy.allocate_contiguous(count)
        }

        fn deallocate_contiguous(
            &mut self,
            first: PhysFrame,
            count: usize,
        ) -> Result<(), FrameError> {
            self.calls += 1;
            self.buddy.deallocate_contiguous(first, count)
        }
    }

    fn buddy(storage: &mut Vec<u64>, frames: usize) -> BuddyAllocator<'_> {
        *storage = vec![0; BuddyAllocator::words_needed(frames)];
        let mut buddy = BuddyAllocator::new(storage, PhysAddr::new(0), frames).unwrap();
        buddy.release_range(PhysAddr::new(0), frames as u64 * FRAME_SIZE);
        buddy
    }

    #[test]
    fn first_allocation_refills_half_the_cache_and_the_rest_never_touch_the_buddy() {
        let mut storage = Vec::new();
        let mut inner = buddy(&mut storage, 256);
        let mut backing = Counting {
            buddy: &mut inner,
            calls: 0,
        };
        let mut cache = FrameCache::<8>::new();
        assert!(cache.allocate(&mut backing).is_some());
        assert_eq!((backing.calls, cache.len()), (4, 3));
        for _ in 0..3 {
            assert!(cache.allocate(&mut backing).is_some());
        }
        assert_eq!(backing.calls, 4);
        assert!(cache.is_empty());
    }

    #[test]
    fn freeing_into_a_cache_with_room_never_touches_the_buddy() {
        let mut storage = Vec::new();
        let mut inner = buddy(&mut storage, 256);
        let mut backing = Counting {
            buddy: &mut inner,
            calls: 0,
        };
        let mut cache = FrameCache::<8>::new();
        let frames: Vec<_> = (0..4)
            .map(|_| cache.allocate(&mut backing).unwrap())
            .collect();
        let calls = backing.calls;
        for frame in frames {
            cache.deallocate(frame, &mut backing).unwrap();
        }
        assert_eq!((backing.calls, cache.len()), (calls, 4));
    }

    #[test]
    fn a_full_cache_returns_its_upper_half_in_one_batch() {
        let mut storage = Vec::new();
        let mut inner = buddy(&mut storage, 256);
        let mut backing = Counting {
            buddy: &mut inner,
            calls: 0,
        };
        let mut cache = FrameCache::<8>::new();
        let frames: Vec<_> = (0..12)
            .map(|_| cache.allocate(&mut backing).unwrap())
            .collect();
        let (before, free) = (backing.calls, backing.buddy.free_frames());
        for frame in frames {
            cache.deallocate(frame, &mut backing).unwrap();
        }
        assert_eq!(backing.calls - before, 4);
        assert_eq!(cache.len(), 8);
        assert_eq!(backing.buddy.free_frames(), free + 4);
    }

    #[test]
    fn flush_gives_everything_back_and_the_buddy_merges_again() {
        let mut storage = Vec::new();
        let mut inner = buddy(&mut storage, 256);
        let mut cache = FrameCache::<16>::new();
        let frame = cache.allocate(&mut inner).unwrap();
        cache.deallocate(frame, &mut inner).unwrap();
        assert_eq!(inner.free_frames(), 256 - 8);
        assert_eq!(cache.flush(&mut inner), Ok(()));
        assert_eq!(
            (inner.free_frames(), inner.largest_free_order()),
            (256, Some(8))
        );
        assert!(cache.is_empty());
    }

    #[test]
    fn exhausted_buddy_makes_the_cache_return_nothing() {
        let mut storage = Vec::new();
        let mut inner = buddy(&mut storage, 4);
        let mut cache = FrameCache::<8>::new();
        let got: Vec<_> = core::iter::from_fn(|| cache.allocate(&mut inner)).collect();
        assert_eq!(got.len(), 4);
        assert_eq!(cache.allocate(&mut inner), None);
    }

    #[test]
    fn a_double_free_only_surfaces_when_the_cache_drains() {
        let mut storage = Vec::new();
        let mut inner = buddy(&mut storage, 64);
        let mut cache = FrameCache::<4>::new();
        let frame = cache.allocate(&mut inner).unwrap();
        cache.deallocate(frame, &mut inner).unwrap();
        cache.deallocate(frame, &mut inner).unwrap();
        assert_eq!(
            cache.flush(&mut inner),
            Err(FrameError::DoubleFree(frame.start()))
        );
    }

    #[test]
    fn view_routes_single_frames_through_the_cache_and_runs_to_the_buddy() {
        let mut storage = Vec::new();
        let mut inner = buddy(&mut storage, 256);
        let mut cache = FrameCache::<8>::new();
        let mut view = cache.through(&mut inner);
        let run = view.allocate_contiguous(3).unwrap();
        let frame = view.allocate_frame().unwrap();
        assert_ne!(run.start(), frame);
        assert_eq!(view.deallocate_contiguous(run, 3), Ok(()));
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn caches_are_independent_per_cpu() {
        let mut storage = Vec::new();
        let mut inner = buddy(&mut storage, 256);
        let mut caches = FrameCaches::<4, 8>::new();
        let a = caches.cpu(0).unwrap().allocate(&mut inner).unwrap();
        let b = caches.cpu(3).unwrap().allocate(&mut inner).unwrap();
        assert_ne!(a, b);
        assert_eq!(caches.cpu(0).unwrap().len(), 3);
        assert!(caches.cpu(4).is_none());
        assert_eq!(caches.as_mut_slice().len(), 4);
    }
}
