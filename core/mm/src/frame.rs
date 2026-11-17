//! Alocador de frames físicos em bitmap: 1 bit por frame de 4 KiB.

use rustos_hal::{FrameSource, PAGE_SIZE, PhysAddr};

pub const FRAME_SIZE: u64 = PAGE_SIZE;

const WORD_BITS: usize = u64::BITS as usize;

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
}

/// Bit em 1 é frame ocupado. Nasce com tudo ocupado: só vira livre o que o
/// mapa de memória disser que é utilizável.
#[derive(Debug)]
pub struct BitmapFrameAllocator<'a> {
    words: &'a mut [u64],
    base: PhysAddr,
    frames: usize,
    free: usize,
    hint: usize,
}

impl<'a> BitmapFrameAllocator<'a> {
    pub const fn words_needed(frames: usize) -> usize {
        frames.div_ceil(WORD_BITS)
    }

    pub fn new(words: &'a mut [u64], base: PhysAddr, frames: usize) -> Result<Self, FrameError> {
        if !base.is_aligned(FRAME_SIZE) {
            return Err(FrameError::MisalignedBase(base));
        }
        let needed_words = Self::words_needed(frames);
        if words.len() < needed_words {
            return Err(FrameError::StorageTooSmall {
                needed_words,
                given_words: words.len(),
            });
        }
        let words = &mut words[..needed_words];
        words.fill(u64::MAX);
        Ok(Self {
            words,
            base,
            frames,
            free: 0,
            hint: 0,
        })
    }

    pub const fn total_frames(&self) -> usize {
        self.frames
    }

    pub const fn free_frames(&self) -> usize {
        self.free
    }

    /// Libera os frames inteiramente contidos em `[start, start + len)`.
    pub fn release_range(&mut self, start: PhysAddr, len: u64) {
        let end = start.as_u64().saturating_add(len);
        if let Some((first, last)) = self.span(start.as_u64(), end, Rounding::Inner) {
            self.free += self.update(first, last, false);
            self.hint = self.hint.min(first / WORD_BITS);
        }
    }

    /// Ocupa todo frame que toque `[start, start + len)`.
    pub fn reserve_range(&mut self, start: PhysAddr, len: u64) {
        let end = start.as_u64().saturating_add(len);
        if let Some((first, last)) = self.span(start.as_u64(), end, Rounding::Outer) {
            self.free -= self.update(first, last, true);
        }
    }

    pub fn allocate(&mut self) -> Option<PhysFrame> {
        let count = self.words.len();
        for step in 0..count {
            let word_index = (self.hint + step) % count;
            let word = self.words[word_index];
            if word == u64::MAX {
                continue;
            }
            let bit = (!word).trailing_zeros() as usize;
            let index = word_index * WORD_BITS + bit;
            if index >= self.frames {
                continue;
            }
            self.words[word_index] |= 1 << bit;
            self.free -= 1;
            self.hint = word_index;
            return Some(self.frame_at(index));
        }
        None
    }

    /// Os `count` frames livres seguidos de endereço mais baixo, numa varredura O(frames).
    /// Para estruturas de boot (grade do terminal); o caminho quente usa `allocate`.
    pub fn allocate_contiguous(&mut self, count: usize) -> Option<PhysFrame> {
        if count == 0 || count > self.free {
            return None;
        }
        let (mut run_start, mut run) = (0, 0);
        let mut index = 0;
        while index < self.frames {
            let word = self.words[index / WORD_BITS];
            if index.is_multiple_of(WORD_BITS) && word == u64::MAX {
                run = 0;
                index += WORD_BITS;
                continue;
            }
            if word & (1 << (index % WORD_BITS)) == 0 {
                if run == 0 {
                    run_start = index;
                }
                run += 1;
                if run == count {
                    self.update(run_start, run_start + count, true);
                    self.free -= count;
                    return Some(self.frame_at(run_start));
                }
            } else {
                run = 0;
            }
            index += 1;
        }
        None
    }

    pub fn deallocate(&mut self, frame: PhysFrame) -> Result<(), FrameError> {
        let index = self.index_of(frame.start())?;
        let (word, bit) = (index / WORD_BITS, index % WORD_BITS);
        if self.words[word] & (1 << bit) == 0 {
            return Err(FrameError::DoubleFree(frame.start()));
        }
        self.words[word] &= !(1 << bit);
        self.free += 1;
        self.hint = self.hint.min(word);
        Ok(())
    }

    fn frame_at(&self, index: usize) -> PhysFrame {
        PhysFrame(PhysAddr::new(
            self.base.as_u64() + index as u64 * FRAME_SIZE,
        ))
    }

    fn index_of(&self, addr: PhysAddr) -> Result<usize, FrameError> {
        let offset = addr
            .as_u64()
            .checked_sub(self.base.as_u64())
            .ok_or(FrameError::OutOfRange(addr))?;
        let index =
            usize::try_from(offset / FRAME_SIZE).map_err(|_| FrameError::OutOfRange(addr))?;
        if index >= self.frames {
            return Err(FrameError::OutOfRange(addr));
        }
        Ok(index)
    }

    /// Converte bytes em índices de frame `[first, last)`, recortados ao alcance do bitmap.
    fn span(&self, start: u64, end: u64, rounding: Rounding) -> Option<(usize, usize)> {
        let base = self.base.as_u64();
        let limit = base.saturating_add(self.frames as u64 * FRAME_SIZE);
        let (start, end) = (start.max(base) - base, end.min(limit).saturating_sub(base));
        if start >= end {
            return None;
        }
        let (first, last) = match rounding {
            Rounding::Inner => (start.div_ceil(FRAME_SIZE), end / FRAME_SIZE),
            Rounding::Outer => (start / FRAME_SIZE, end.div_ceil(FRAME_SIZE)),
        };
        let first = usize::try_from(first).ok()?;
        let last = usize::try_from(last).ok()?;
        (first < last).then_some((first, last))
    }

    /// Marca `[first, last)` palavra a palavra; devolve quantos bits mudaram.
    fn update(&mut self, first: usize, last: usize, used: bool) -> usize {
        let mut changed = 0;
        let mut index = first;
        while index < last {
            let word_index = index / WORD_BITS;
            let low = index % WORD_BITS;
            let high = (last - word_index * WORD_BITS).min(WORD_BITS);
            let mask = bit_mask(low, high);
            let word = &mut self.words[word_index];
            if used {
                changed += (!*word & mask).count_ones() as usize;
                *word |= mask;
            } else {
                changed += (*word & mask).count_ones() as usize;
                *word &= !mask;
            }
            index = (word_index + 1) * WORD_BITS;
        }
        changed
    }
}

impl FrameSource for BitmapFrameAllocator<'_> {
    fn allocate_frame(&mut self) -> Option<PhysAddr> {
        self.allocate().map(PhysFrame::start)
    }
}

#[derive(Clone, Copy)]
enum Rounding {
    Inner,
    Outer,
}

/// Bits `[low, high)` ligados.
const fn bit_mask(low: usize, high: usize) -> u64 {
    let width = high - low;
    if width == WORD_BITS {
        u64::MAX
    } else {
        ((1u64 << width) - 1) << low
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u64 = 0x10_0000;

    fn allocator(storage: &mut [u64], frames: usize) -> BitmapFrameAllocator<'_> {
        BitmapFrameAllocator::new(storage, PhysAddr::new(BASE), frames).unwrap()
    }

    fn frame(index: u64) -> PhysAddr {
        PhysAddr::new(BASE + index * FRAME_SIZE)
    }

    #[test]
    fn starts_with_every_frame_used() {
        let mut storage = [0; 2];
        let mut frames = allocator(&mut storage, 100);
        assert_eq!(frames.free_frames(), 0);
        assert_eq!(frames.allocate(), None);
    }

    #[test]
    fn rejects_storage_smaller_than_bitmap() {
        let mut storage = [0; 1];
        let result = BitmapFrameAllocator::new(&mut storage, PhysAddr::new(BASE), 65);
        assert_eq!(
            result.unwrap_err(),
            FrameError::StorageTooSmall {
                needed_words: 2,
                given_words: 1
            }
        );
    }

    #[test]
    fn rejects_misaligned_base() {
        let mut storage = [0; 1];
        let result = BitmapFrameAllocator::new(&mut storage, PhysAddr::new(BASE + 1), 1);
        assert_eq!(
            result.unwrap_err(),
            FrameError::MisalignedBase(PhysAddr::new(BASE + 1))
        );
    }

    #[test]
    fn allocates_lowest_released_frame_first() {
        let mut storage = [0; 2];
        let mut frames = allocator(&mut storage, 100);
        frames.release_range(frame(10), 5 * FRAME_SIZE);
        assert_eq!(frames.free_frames(), 5);
        assert_eq!(frames.allocate().map(PhysFrame::start), Some(frame(10)));
        assert_eq!(frames.allocate().map(PhysFrame::start), Some(frame(11)));
        assert_eq!(frames.free_frames(), 3);
    }

    #[test]
    fn release_ignores_partial_frames() {
        let mut storage = [0; 1];
        let mut frames = allocator(&mut storage, 10);
        frames.release_range(PhysAddr::new(BASE + 1), 2 * FRAME_SIZE);
        assert_eq!(frames.free_frames(), 1);
        assert_eq!(frames.allocate().map(PhysFrame::start), Some(frame(1)));
    }

    #[test]
    fn reserve_covers_partial_frames() {
        let mut storage = [0; 1];
        let mut frames = allocator(&mut storage, 10);
        frames.release_range(frame(0), 10 * FRAME_SIZE);
        frames.reserve_range(PhysAddr::new(BASE + FRAME_SIZE - 1), 2);
        assert_eq!(frames.free_frames(), 8);
    }

    #[test]
    fn ranges_are_clipped_to_tracked_memory() {
        let mut storage = [0; 1];
        let mut frames = allocator(&mut storage, 4);
        frames.release_range(PhysAddr::new(0), BASE + 100 * FRAME_SIZE);
        assert_eq!(frames.free_frames(), 4);
    }

    #[test]
    fn release_spanning_many_words_counts_every_frame() {
        let mut storage = [0; 4];
        let mut frames = allocator(&mut storage, 200);
        frames.release_range(frame(3), 190 * FRAME_SIZE);
        assert_eq!(frames.free_frames(), 190);
        frames.reserve_range(frame(0), 200 * FRAME_SIZE);
        assert_eq!(frames.free_frames(), 0);
    }

    #[test]
    fn never_hands_out_bits_past_the_last_frame() {
        let mut storage = [0; 1];
        let mut frames = allocator(&mut storage, 3);
        frames.release_range(frame(0), 64 * FRAME_SIZE);
        assert_eq!(frames.free_frames(), 3);
        for _ in 0..3 {
            assert!(frames.allocate().is_some());
        }
        assert_eq!(frames.allocate(), None);
    }

    #[test]
    fn deallocate_returns_frame_and_detects_double_free() {
        let mut storage = [0; 1];
        let mut frames = allocator(&mut storage, 8);
        frames.release_range(frame(0), 8 * FRAME_SIZE);
        let taken = frames.allocate().unwrap();
        assert_eq!(frames.deallocate(taken), Ok(()));
        assert_eq!(
            frames.deallocate(taken),
            Err(FrameError::DoubleFree(taken.start()))
        );
        assert_eq!(frames.free_frames(), 8);
    }

    #[test]
    fn deallocate_rejects_foreign_frames() {
        let mut storage = [0; 1];
        let mut frames = allocator(&mut storage, 8);
        let foreign = PhysFrame::containing(PhysAddr::new(0));
        assert_eq!(
            frames.deallocate(foreign),
            Err(FrameError::OutOfRange(PhysAddr::new(0)))
        );
    }

    #[test]
    fn freed_frame_is_reused_before_higher_ones() {
        let mut storage = [0; 2];
        let mut frames = allocator(&mut storage, 128);
        frames.release_range(frame(0), FRAME_SIZE);
        frames.release_range(frame(100), FRAME_SIZE);
        assert_eq!(frames.allocate().map(PhysFrame::start), Some(frame(0)));
        assert_eq!(frames.allocate().map(PhysFrame::start), Some(frame(100)));
        frames.deallocate(PhysFrame::containing(frame(0))).unwrap();
        assert_eq!(frames.allocate().map(PhysFrame::start), Some(frame(0)));
    }

    #[test]
    fn contiguous_run_may_cross_a_word_boundary() {
        let mut storage = [0; 3];
        let mut frames = allocator(&mut storage, 150);
        frames.release_range(frame(10), 3 * FRAME_SIZE);
        frames.release_range(frame(60), 10 * FRAME_SIZE);
        let run = frames
            .allocate_contiguous(8)
            .expect("há 10 seguidos a partir de 60");
        assert_eq!(run.start(), frame(60));
        assert_eq!(frames.free_frames(), 5);
        assert_eq!(
            frames.allocate_contiguous(3).map(PhysFrame::start),
            Some(frame(10))
        );
    }

    #[test]
    fn contiguous_frames_are_marked_used() {
        let mut storage = [0; 1];
        let mut frames = allocator(&mut storage, 8);
        frames.release_range(frame(0), 8 * FRAME_SIZE);
        assert!(frames.allocate_contiguous(6).is_some());
        assert_eq!(frames.allocate().map(PhysFrame::start), Some(frame(6)));
        assert_eq!(frames.allocate().map(PhysFrame::start), Some(frame(7)));
        assert_eq!(frames.allocate(), None);
    }

    #[test]
    fn contiguous_rejects_zero_too_many_and_fragmented() {
        let mut storage = [0; 1];
        let mut frames = allocator(&mut storage, 10);
        frames.release_range(frame(0), 2 * FRAME_SIZE);
        frames.release_range(frame(5), 2 * FRAME_SIZE);
        assert_eq!(frames.allocate_contiguous(0), None);
        assert_eq!(frames.allocate_contiguous(5), None);
        assert_eq!(frames.allocate_contiguous(3), None);
        assert_eq!(frames.free_frames(), 4);
    }

    #[test]
    fn contiguous_never_runs_past_the_last_frame() {
        let mut storage = [0; 1];
        let mut frames = allocator(&mut storage, 4);
        frames.release_range(frame(0), 4 * FRAME_SIZE);
        assert_eq!(frames.allocate_contiguous(5), None);
        assert_eq!(
            frames.allocate_contiguous(4).map(PhysFrame::start),
            Some(frame(0))
        );
    }

    #[test]
    fn bit_mask_covers_requested_bits() {
        assert_eq!(bit_mask(0, 64), u64::MAX);
        assert_eq!(bit_mask(4, 8), 0xf0);
    }
}
