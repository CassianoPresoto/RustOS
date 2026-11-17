//! Alocador de frames em bitmap (1 bit por frame, varredura): fora do kernel desde que o buddy
//! entrou. Fica só como base de comparação do `bench`.

use rustos_hal::PhysAddr;

use crate::frame::{FRAME_SIZE, FrameError, PhysFrame};

const WORD_BITS: usize = u64::BITS as usize;

/// Bit em 1 é frame ocupado. Nasce com tudo ocupado: só vira livre o que o
/// mapa de memória disser que é utilizável.
#[derive(Debug)]
pub(crate) struct BitmapFrameAllocator<'a> {
    words: &'a mut [u64],
    base: PhysAddr,
    frames: usize,
    free: usize,
    hint: usize,
}

impl<'a> BitmapFrameAllocator<'a> {
    pub(crate) const fn words_needed(frames: usize) -> usize {
        frames.div_ceil(WORD_BITS)
    }

    pub(crate) fn new(
        words: &'a mut [u64],
        base: PhysAddr,
        frames: usize,
    ) -> Result<Self, FrameError> {
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

    /// Libera os frames inteiramente contidos em `[start, start + len)`.
    pub(crate) fn release_range(&mut self, start: PhysAddr, len: u64) {
        let end = start.as_u64().saturating_add(len);
        if let Some((first, last)) = self.span(start.as_u64(), end) {
            self.free += self.update(first, last, false);
            self.hint = self.hint.min(first / WORD_BITS);
        }
    }

    pub(crate) fn allocate(&mut self) -> Option<PhysFrame> {
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
    pub(crate) fn allocate_contiguous(&mut self, count: usize) -> Option<PhysFrame> {
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

    pub(crate) fn deallocate(&mut self, frame: PhysFrame) -> Result<(), FrameError> {
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
        PhysFrame::containing(PhysAddr::new(
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
    fn span(&self, start: u64, end: u64) -> Option<(usize, usize)> {
        let base = self.base.as_u64();
        let limit = base.saturating_add(self.frames as u64 * FRAME_SIZE);
        let (start, end) = (start.max(base) - base, end.min(limit).saturating_sub(base));
        if start >= end {
            return None;
        }
        let (first, last) = (start.div_ceil(FRAME_SIZE), end / FRAME_SIZE);
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

/// Bits `[low, high)` ligados.
const fn bit_mask(low: usize, high: usize) -> u64 {
    let width = high - low;
    if width == WORD_BITS {
        u64::MAX
    } else {
        ((1u64 << width) - 1) << low
    }
}
