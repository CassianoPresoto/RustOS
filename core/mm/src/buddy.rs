//! Alocador buddy de frames: blocos de 2^k frames alinhados a 2^k, sem tocar a memória deles.

use rustos_hal::{FrameSource, PhysAddr};

use crate::bittree::{BitTree, words_for};
use crate::frame::{FRAME_SIZE, FrameError, PhysFrame};

/// Maior bloco: 2^18 frames = 1 GiB, o tamanho da página grande do x86_64 e do aarch64.
pub const MAX_ORDER: usize = 18;
const ORDERS: usize = MAX_ORDER + 1;

/// Fonte de frames em corridas contíguas, em ordem alinhada; o heap e o cache falam com ela.
pub trait FrameAllocator {
    fn allocate_contiguous(&mut self, count: usize) -> Option<PhysFrame>;
    fn deallocate_contiguous(&mut self, first: PhysFrame, count: usize) -> Result<(), FrameError>;
    /// Confere, sem mudar nada, que `deallocate_contiguous(first, count)` aceitaria a
    /// corrida: quem vai zerar os frames chama antes, para não zerar memória livre.
    fn verify_used(&self, first: PhysFrame, count: usize) -> Result<(), FrameError>;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Overlap {
    None,
    /// O bloco inteiro já está livre, dentro de um bloco livre igual ou maior.
    Free,
    /// Parte dos frames do bloco já está livre.
    Partial,
}

/// Um bit por bloco de cada ordem: ligado é "livre e não fundido num pai". O
/// alocador nasce com tudo ocupado; `release_range` é que libera.
#[derive(Debug)]
pub struct BuddyAllocator<'a> {
    store: &'a mut [u64],
    trees: [BitTree; ORDERS],
    /// Bit `k` ligado: há bloco livre de ordem `k`. Poupa olhar árvore vazia.
    nonempty: u32,
    base: usize,
    frames: usize,
    free: usize,
}

impl<'a> BuddyAllocator<'a> {
    /// Limite superior, para qualquer `base`, das palavras que `new` precisa.
    pub const fn words_needed(frames: usize) -> usize {
        let mut total = 0;
        let mut order = 0;
        while order < ORDERS {
            total += words_for(frames.div_ceil(1 << order) + 1);
            order += 1;
        }
        total
    }

    pub fn new(store: &'a mut [u64], base: PhysAddr, frames: usize) -> Result<Self, FrameError> {
        if !base.is_aligned(FRAME_SIZE) {
            return Err(FrameError::MisalignedBase(base));
        }
        let base = usize::try_from(base.as_u64() / FRAME_SIZE)
            .map_err(|_| FrameError::OutOfRange(PhysAddr::new(base.as_u64())))?;
        let mut trees = [BitTree::EMPTY; ORDERS];
        let mut needed = 0;
        for (order, tree) in trees.iter_mut().enumerate() {
            let blocks = Self::blocks_at(base, frames, order);
            *tree = BitTree::at(needed, blocks).ok_or(FrameError::TooManyFrames(frames))?;
            needed += words_for(blocks);
        }
        if store.len() < needed {
            return Err(FrameError::StorageTooSmall {
                needed_words: needed,
                given_words: store.len(),
            });
        }
        let (store, _) = store.split_at_mut(needed);
        store.fill(0);
        Ok(Self {
            store,
            trees,
            nonempty: 0,
            base,
            frames,
            free: 0,
        })
    }

    pub const fn total_frames(&self) -> usize {
        self.frames
    }

    pub const fn free_frames(&self) -> usize {
        self.free
    }

    /// Ordem do maior bloco livre, ou `None` sem memória livre.
    pub fn largest_free_order(&self) -> Option<usize> {
        (self.nonempty != 0).then(|| self.nonempty.ilog2() as usize)
    }

    pub fn allocate(&mut self) -> Option<PhysFrame> {
        self.allocate_order(0)
    }

    pub fn deallocate(&mut self, frame: PhysFrame) -> Result<(), FrameError> {
        self.deallocate_order(frame, 0)
    }

    /// Bloco de 2^`order` frames, o de menor endereço entre os da menor ordem que serve.
    pub fn allocate_order(&mut self, order: usize) -> Option<PhysFrame> {
        if order > MAX_ORDER || (1 << order) > self.free {
            return None;
        }
        self.take_block(order).map(frame_to_phys)
    }

    pub fn deallocate_order(&mut self, frame: PhysFrame, order: usize) -> Result<(), FrameError> {
        if order > MAX_ORDER {
            return Err(FrameError::OrderTooLarge(order));
        }
        let first = self.number_of(frame.start(), 1 << order)?;
        if !first.is_multiple_of(1 << order) {
            return Err(FrameError::MisalignedBlock(frame.start()));
        }
        match self.overlap(first, order) {
            Overlap::None => {
                self.insert_free(first, order);
                Ok(())
            }
            _ => Err(FrameError::DoubleFree(frame.start())),
        }
    }

    /// `count` frames seguidos, no começo de um bloco da menor ordem que os contém; a
    /// sobra do bloco volta à lista.
    pub fn allocate_contiguous(&mut self, count: usize) -> Option<PhysFrame> {
        if count == 0 || count > self.free {
            return None;
        }
        let order = count.next_power_of_two().trailing_zeros() as usize;
        if order > MAX_ORDER {
            return None;
        }
        let first = self.take_block(order)?;
        self.insert_range(first + count, first + (1 << order));
        Some(frame_to_phys(first))
    }

    /// Devolve uma corrida de `allocate_contiguous`, ou qualquer corrida de frames ocupados.
    pub fn deallocate_contiguous(
        &mut self,
        first: PhysFrame,
        count: usize,
    ) -> Result<(), FrameError> {
        self.verify_used(first, count)?;
        let start = self.number_of(first.start(), count.max(1))?;
        self.insert_range(start, start + count);
        Ok(())
    }

    /// Todo frame da corrida está rastreado e ocupado (nenhum livre, nem dentro de bloco maior).
    pub fn verify_used(&self, first: PhysFrame, count: usize) -> Result<(), FrameError> {
        let start = self.number_of(first.start(), count.max(1))?;
        let end = start + count;
        let mut frame = start;
        while frame < end {
            let order = fit_order(frame, end);
            if self.overlap(frame, order) != Overlap::None {
                return Err(FrameError::DoubleFree(frame_to_phys(frame).start()));
            }
            frame += 1 << order;
        }
        Ok(())
    }

    /// Libera os frames inteiramente contidos em `[start, start + len)`; os já livres ficam.
    pub fn release_range(&mut self, start: PhysAddr, len: u64) {
        if let Some((first, last)) = self.span(start, len, true) {
            let mut frame = first;
            while frame < last {
                let order = fit_order(frame, last);
                self.release_block(frame, order);
                frame += 1 << order;
            }
        }
    }

    /// Ocupa todo frame que toque `[start, start + len)`, partindo os blocos livres que o cobrem.
    pub fn reserve_range(&mut self, start: PhysAddr, len: u64) {
        let Some((first, last)) = self.span(start, len, false) else {
            return;
        };
        let mut frame = first;
        while frame < last && self.nonempty != 0 {
            let Some((head, order)) = self.free_block_containing(frame) else {
                frame += 1;
                continue;
            };
            let tail = head + (1 << order);
            self.clear_free(head, order);
            self.free -= 1 << order;
            self.insert_range(head, first.max(head));
            self.insert_range(last.min(tail).max(head), tail);
            frame = tail;
        }
    }

    fn take_block(&mut self, order: usize) -> Option<usize> {
        let above = self.nonempty >> order;
        if above == 0 {
            return None;
        }
        let found = order + above.trailing_zeros() as usize;
        let index = self.trees[found].first_set(self.store)?;
        let frame = ((self.base >> found) + index) << found;
        self.clear_free(frame, found);
        for lower in (order..found).rev() {
            self.set_free(frame + (1 << lower), lower);
        }
        self.free -= 1 << order;
        Some(frame)
    }

    /// Marca livre o bloco e o funde com o irmão enquanto o irmão também estiver livre.
    fn insert_free(&mut self, mut frame: usize, mut order: usize) {
        self.free += 1 << order;
        while order < MAX_ORDER {
            let buddy = frame ^ (1 << order);
            if !self.contains(buddy, order) {
                break;
            }
            if !self.trees[order].test(self.store, self.index(buddy, order)) {
                break;
            }
            self.clear_free(buddy, order);
            frame &= !(1 << order);
            order += 1;
        }
        self.set_free(frame, order);
    }

    /// Libera `[first, last)` em blocos alinhados; os frames estão ocupados.
    fn insert_range(&mut self, first: usize, last: usize) {
        let mut frame = first;
        while frame < last {
            let order = fit_order(frame, last);
            self.insert_free(frame, order);
            frame += 1 << order;
        }
    }

    fn release_block(&mut self, frame: usize, order: usize) {
        match self.overlap(frame, order) {
            Overlap::None => self.insert_free(frame, order),
            Overlap::Free => {}
            Overlap::Partial => {
                self.release_block(frame, order - 1);
                self.release_block(frame + (1 << (order - 1)), order - 1);
            }
        }
    }

    fn overlap(&self, frame: usize, order: usize) -> Overlap {
        let mut above = self.nonempty & (u32::MAX << order);
        while above != 0 {
            let level = above.trailing_zeros() as usize;
            above &= above - 1;
            let head = frame & !((1 << level) - 1);
            if self.contains(head, level)
                && self.trees[level].test(self.store, self.index(head, level))
            {
                return Overlap::Free;
            }
        }
        let mut below = self.nonempty & ((1 << order) - 1);
        while below != 0 {
            let level = below.trailing_zeros() as usize;
            below &= below - 1;
            let low = self.index(frame, level);
            if self.trees[level].any_in(self.store, low, low + (1 << (order - level))) {
                return Overlap::Partial;
            }
        }
        Overlap::None
    }

    fn free_block_containing(&self, frame: usize) -> Option<(usize, usize)> {
        let mut orders = self.nonempty;
        while orders != 0 {
            let order = orders.trailing_zeros() as usize;
            orders &= orders - 1;
            let head = frame & !((1 << order) - 1);
            if self.contains(head, order)
                && self.trees[order].test(self.store, self.index(head, order))
            {
                return Some((head, order));
            }
        }
        None
    }

    fn set_free(&mut self, frame: usize, order: usize) {
        let index = self.index(frame, order);
        self.trees[order].set(self.store, index);
        self.nonempty |= 1 << order;
    }

    fn clear_free(&mut self, frame: usize, order: usize) {
        let index = self.index(frame, order);
        self.trees[order].clear(self.store, index);
        if self.trees[order].is_empty(self.store) {
            self.nonempty &= !(1 << order);
        }
    }

    const fn end(&self) -> usize {
        self.base + self.frames
    }

    const fn contains(&self, frame: usize, order: usize) -> bool {
        frame >= self.base && frame + (1 << order) <= self.end()
    }

    const fn index(&self, frame: usize, order: usize) -> usize {
        (frame >> order) - (self.base >> order)
    }

    const fn blocks_at(base: usize, frames: usize, order: usize) -> usize {
        (base + frames).div_ceil(1 << order) - (base >> order)
    }

    /// Número do frame de `addr`, desde que `len` frames dali caibam na memória rastreada.
    fn number_of(&self, addr: PhysAddr, len: usize) -> Result<usize, FrameError> {
        let frame = usize::try_from(addr.as_u64() / FRAME_SIZE)
            .map_err(|_| FrameError::OutOfRange(addr))?;
        if frame < self.base || frame.checked_add(len).is_none_or(|end| end > self.end()) {
            return Err(FrameError::OutOfRange(addr));
        }
        Ok(frame)
    }

    /// Frames `[first, last)` da memória rastreada que `[start, start + len)` cobre: só os
    /// inteiros se `inner`, todo frame tocado se não.
    fn span(&self, start: PhysAddr, len: u64, inner: bool) -> Option<(usize, usize)> {
        let (begin, end) = (start.as_u64(), start.as_u64().saturating_add(len));
        let (low, high) = if inner {
            (begin.div_ceil(FRAME_SIZE), end / FRAME_SIZE)
        } else {
            (begin / FRAME_SIZE, end.div_ceil(FRAME_SIZE))
        };
        let first = usize::try_from(low).ok()?.max(self.base);
        let last = usize::try_from(high).ok()?.min(self.end());
        (first < last).then_some((first, last))
    }
}

fn frame_to_phys(frame: usize) -> PhysFrame {
    PhysFrame::containing(PhysAddr::new(frame as u64 * FRAME_SIZE))
}

/// Maior ordem de um bloco que começa em `frame`, alinhado a si mesmo, e cabe até `end`.
const fn fit_order(frame: usize, end: usize) -> usize {
    let aligned = if frame == 0 {
        MAX_ORDER
    } else {
        let tz = frame.trailing_zeros() as usize;
        if tz < MAX_ORDER { tz } else { MAX_ORDER }
    };
    let room = (end - frame).ilog2() as usize;
    if aligned < room { aligned } else { room }
}

impl FrameSource for BuddyAllocator<'_> {
    fn allocate_frame(&mut self) -> Option<PhysAddr> {
        self.allocate().map(PhysFrame::start)
    }
}

impl FrameAllocator for BuddyAllocator<'_> {
    fn allocate_contiguous(&mut self, count: usize) -> Option<PhysFrame> {
        BuddyAllocator::allocate_contiguous(self, count)
    }

    fn deallocate_contiguous(&mut self, first: PhysFrame, count: usize) -> Result<(), FrameError> {
        BuddyAllocator::deallocate_contiguous(self, first, count)
    }

    fn verify_used(&self, first: PhysFrame, count: usize) -> Result<(), FrameError> {
        BuddyAllocator::verify_used(self, first, count)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::testing::XorShift;

    /// Uma região a partir do frame `base` com `frames` frames, toda livre.
    fn released(storage: &mut Vec<u64>, base: usize, frames: usize) -> BuddyAllocator<'_> {
        *storage = vec![0; BuddyAllocator::words_needed(frames)];
        let start = PhysAddr::new(base as u64 * FRAME_SIZE);
        let mut buddy = BuddyAllocator::new(storage, start, frames).unwrap();
        buddy.release_range(start, frames as u64 * FRAME_SIZE);
        buddy
    }

    fn number(frame: PhysFrame) -> usize {
        usize::try_from(frame.start().as_u64() / FRAME_SIZE).unwrap()
    }

    fn at(frame: usize) -> PhysFrame {
        PhysFrame::containing(PhysAddr::new(frame as u64 * FRAME_SIZE))
    }

    #[test]
    fn starts_with_every_frame_used() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 0, 64);
        buddy.reserve_range(PhysAddr::new(0), 64 * FRAME_SIZE);
        assert_eq!(buddy.free_frames(), 0);
        assert_eq!(buddy.allocate(), None);
        assert_eq!(buddy.largest_free_order(), None);
        let mut empty = vec![0; BuddyAllocator::words_needed(64)];
        let fresh = BuddyAllocator::new(&mut empty, PhysAddr::new(0), 64).unwrap();
        assert_eq!((fresh.free_frames(), fresh.total_frames()), (0, 64));
    }

    #[test]
    fn rejects_storage_smaller_than_the_trees() {
        let mut storage = [0; 3];
        let error = BuddyAllocator::new(&mut storage, PhysAddr::new(0), 4096).unwrap_err();
        assert!(matches!(
            error,
            FrameError::StorageTooSmall { given_words: 3, .. }
        ));
    }

    #[test]
    fn rejects_misaligned_base_and_oversized_regions() {
        let mut storage = [0; 8];
        assert_eq!(
            BuddyAllocator::new(&mut storage, PhysAddr::new(1), 1).unwrap_err(),
            FrameError::MisalignedBase(PhysAddr::new(1))
        );
        let mut storage = [0; 8];
        assert_eq!(
            BuddyAllocator::new(&mut storage, PhysAddr::new(0), 1 << 31).unwrap_err(),
            FrameError::TooManyFrames(1 << 31)
        );
    }

    #[test]
    fn aligned_region_becomes_one_maximal_block() {
        let mut storage = Vec::new();
        let buddy = released(&mut storage, 1 << 10, 1 << 10);
        assert_eq!(buddy.free_frames(), 1 << 10);
        assert_eq!(buddy.largest_free_order(), Some(10));
    }

    #[test]
    fn region_that_starts_unaligned_is_cut_into_aligned_blocks() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 3, 37);
        assert_eq!(buddy.free_frames(), 37);
        let mut taken = Vec::new();
        for order in [4, 3, 2, 1, 0, 0] {
            let frame = number(buddy.allocate_order(order).unwrap());
            assert_eq!(frame % (1 << order), 0, "bloco de ordem {order} em {frame}");
            assert!((3..40).contains(&frame));
            taken.push((frame, 1 << order));
        }
        taken.sort_unstable();
        assert!(
            taken
                .windows(2)
                .all(|pair| pair[0].0 + pair[0].1 <= pair[1].0)
        );
    }

    #[test]
    fn release_ignores_partial_frames_at_both_ends() {
        let mut storage = vec![0; BuddyAllocator::words_needed(16)];
        let mut buddy = BuddyAllocator::new(&mut storage, PhysAddr::new(0), 16).unwrap();
        buddy.release_range(PhysAddr::new(1), 3 * FRAME_SIZE);
        assert_eq!(buddy.free_frames(), 2);
        assert_eq!(number(buddy.allocate().unwrap()), 1);
        assert_eq!(number(buddy.allocate().unwrap()), 2);
        assert_eq!(buddy.allocate(), None);
    }

    #[test]
    fn release_is_idempotent_and_clipped_to_the_tracked_memory() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 8, 100);
        let before = buddy.free_frames();
        buddy.release_range(PhysAddr::new(0), 1 << 30);
        buddy.release_range(PhysAddr::new(20 * FRAME_SIZE), 5 * FRAME_SIZE);
        assert_eq!((before, buddy.free_frames()), (100, 100));
    }

    #[test]
    fn hands_out_the_lowest_address_of_the_smallest_fitting_order() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 0, 16);
        assert_eq!(number(buddy.allocate().unwrap()), 0);
        assert_eq!(number(buddy.allocate().unwrap()), 1);
        assert_eq!(number(buddy.allocate_order(1).unwrap()), 2);
        assert_eq!(number(buddy.allocate().unwrap()), 4);
        assert_eq!(buddy.free_frames(), 16 - 5);
    }

    #[test]
    fn freeing_one_frame_merges_the_whole_cascade() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 0, 1 << 12);
        assert_eq!(buddy.largest_free_order(), Some(12));
        let frame = buddy.allocate().unwrap();
        assert_eq!(buddy.largest_free_order(), Some(11));
        assert_eq!(buddy.deallocate(frame), Ok(()));
        assert_eq!(buddy.largest_free_order(), Some(12));
        assert_eq!(buddy.free_frames(), 1 << 12);
    }

    #[test]
    fn serves_the_maximum_order_and_refuses_the_next() {
        let frames = 1 << MAX_ORDER;
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 0, frames);
        assert_eq!(buddy.allocate_order(MAX_ORDER + 1), None);
        let block = buddy.allocate_order(MAX_ORDER).unwrap();
        assert_eq!(block.start(), PhysAddr::new(0));
        assert_eq!(buddy.free_frames(), 0);
        assert_eq!(buddy.deallocate_order(block, MAX_ORDER), Ok(()));
        assert_eq!(
            buddy.deallocate_order(block, MAX_ORDER + 1),
            Err(FrameError::OrderTooLarge(MAX_ORDER + 1))
        );
    }

    #[test]
    fn maximum_order_block_must_be_aligned_in_absolute_terms() {
        let frames = 3 << MAX_ORDER;
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 1 << (MAX_ORDER - 1), frames);
        let block = buddy.allocate_order(MAX_ORDER).unwrap();
        assert_eq!(number(block), 1 << MAX_ORDER);
        assert_eq!(buddy.free_frames(), frames - (1 << MAX_ORDER));
    }

    #[test]
    fn detects_double_free_of_a_frame_a_block_and_a_part_of_a_free_block() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 0, 64);
        let frame = buddy.allocate().unwrap();
        assert_eq!(buddy.deallocate(frame), Ok(()));
        assert_eq!(
            buddy.deallocate(frame),
            Err(FrameError::DoubleFree(frame.start()))
        );
        let block = buddy.allocate_order(3).unwrap();
        assert_eq!(buddy.deallocate_order(block, 3), Ok(()));
        assert_eq!(
            buddy.deallocate_order(block, 3),
            Err(FrameError::DoubleFree(block.start()))
        );
        assert_eq!(
            buddy.deallocate(at(5)),
            Err(FrameError::DoubleFree(at(5).start()))
        );
        assert_eq!(buddy.free_frames(), 64);
    }

    #[test]
    fn verify_used_answers_like_the_free_would_without_changing_anything() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 16, 16);
        let run = buddy.allocate_contiguous(5).unwrap();
        assert_eq!(buddy.verify_used(run, 5), Ok(()));
        assert_eq!(buddy.free_frames(), 11);
        assert_eq!(
            buddy.verify_used(run, 8),
            Err(FrameError::DoubleFree(run.start()))
        );
        assert_eq!(
            buddy.verify_used(at(8), 1),
            Err(FrameError::OutOfRange(at(8).start()))
        );
        assert_eq!(buddy.deallocate_contiguous(run, 5), Ok(()));
        assert_eq!(
            buddy.verify_used(run, 5),
            Err(FrameError::DoubleFree(run.start()))
        );
        assert_eq!(buddy.free_frames(), 16);
    }

    #[test]
    fn detects_a_big_block_freed_over_free_small_ones() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 0, 16);
        let all = buddy.allocate_order(4).unwrap();
        assert_eq!(buddy.deallocate(at(3)), Ok(()));
        assert_eq!(
            buddy.deallocate_order(all, 4),
            Err(FrameError::DoubleFree(all.start()))
        );
        assert_eq!(buddy.free_frames(), 1);
    }

    #[test]
    fn rejects_misaligned_and_foreign_blocks() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 16, 16);
        let _ = buddy.allocate_order(4).unwrap();
        assert_eq!(
            buddy.deallocate_order(at(18), 2),
            Err(FrameError::MisalignedBlock(at(18).start()))
        );
        assert_eq!(
            buddy.deallocate(at(2)),
            Err(FrameError::OutOfRange(at(2).start()))
        );
        assert_eq!(
            buddy.deallocate(at(32)),
            Err(FrameError::OutOfRange(at(32).start()))
        );
        assert_eq!(
            buddy.deallocate_order(at(16), 5),
            Err(FrameError::OutOfRange(at(16).start()))
        );
    }

    #[test]
    fn reserve_splits_free_blocks_and_never_hands_the_reserved_frames_out() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 0, 64);
        buddy.reserve_range(PhysAddr::new(10 * FRAME_SIZE + 1), 2 * FRAME_SIZE);
        assert_eq!(buddy.free_frames(), 64 - 3);
        let mut seen = Vec::new();
        while let Some(frame) = buddy.allocate() {
            seen.push(number(frame));
        }
        assert_eq!(seen.len(), 61);
        assert!(!seen.iter().any(|frame| (10..13).contains(frame)));
    }

    #[test]
    fn reserve_of_used_or_foreign_memory_changes_nothing() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 4, 8);
        buddy.reserve_range(PhysAddr::new(0), 2 * FRAME_SIZE);
        buddy.reserve_range(PhysAddr::new(100 * FRAME_SIZE), FRAME_SIZE);
        assert_eq!(buddy.free_frames(), 8);
        buddy.reserve_range(PhysAddr::new(4 * FRAME_SIZE), 8 * FRAME_SIZE);
        buddy.reserve_range(PhysAddr::new(4 * FRAME_SIZE), 8 * FRAME_SIZE);
        assert_eq!(buddy.free_frames(), 0);
    }

    #[test]
    fn reserved_range_can_be_released_and_merges_back() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 0, 256);
        buddy.reserve_range(PhysAddr::new(77 * FRAME_SIZE), 5 * FRAME_SIZE);
        assert_eq!(buddy.largest_free_order(), Some(7));
        buddy.release_range(PhysAddr::new(77 * FRAME_SIZE), 5 * FRAME_SIZE);
        assert_eq!(
            (buddy.free_frames(), buddy.largest_free_order()),
            (256, Some(8))
        );
    }

    #[test]
    fn checkerboard_fragmentation_blocks_larger_requests_until_the_rest_is_freed() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 0, 256);
        let frames: Vec<_> = (0..256).map(|_| buddy.allocate().unwrap()).collect();
        for frame in frames.iter().step_by(2) {
            buddy.deallocate(*frame).unwrap();
        }
        assert_eq!(
            (buddy.free_frames(), buddy.largest_free_order()),
            (128, Some(0))
        );
        assert_eq!(buddy.allocate_order(1), None);
        assert_eq!(buddy.allocate_contiguous(2), None);
        for frame in frames.iter().skip(1).step_by(2) {
            buddy.deallocate(*frame).unwrap();
        }
        assert_eq!(buddy.largest_free_order(), Some(8));
        assert!(buddy.allocate_order(8).is_some());
    }

    #[test]
    fn contiguous_run_of_odd_length_returns_the_tail_of_its_block() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 0, 64);
        let run = buddy.allocate_contiguous(5).unwrap();
        assert_eq!((number(run), buddy.free_frames()), (0, 59));
        assert_eq!(buddy.allocate_contiguous(0), None);
        assert_eq!(buddy.allocate_contiguous(65), None);
        assert_eq!(buddy.deallocate_contiguous(run, 5), Ok(()));
        assert_eq!(
            (buddy.free_frames(), buddy.largest_free_order()),
            (64, Some(6))
        );
        assert_eq!(
            buddy.deallocate_contiguous(run, 5),
            Err(FrameError::DoubleFree(run.start()))
        );
        assert_eq!(buddy.free_frames(), 64);
    }

    #[test]
    fn contiguous_run_is_refused_when_only_fragments_remain() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 0, 8);
        let _ = buddy.allocate_order(3).unwrap();
        buddy.deallocate_contiguous(at(1), 2).unwrap();
        buddy.deallocate_contiguous(at(5), 2).unwrap();
        assert_eq!(buddy.free_frames(), 4);
        assert_eq!(buddy.allocate_contiguous(3), None);
        assert_eq!(buddy.allocate_contiguous(2), None);
        assert_eq!(buddy.allocate().map(number), Some(1));
    }

    #[test]
    fn frame_source_hands_out_physical_addresses() {
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, 2, 2);
        assert_eq!(buddy.allocate_frame(), Some(at(2).start()));
    }

    #[test]
    fn storage_bound_covers_every_base_alignment() {
        for frames in [1, 7, 64, 1000, 65_536, 262_144 + 5] {
            for base in [0, 1, 3, 511, 512, 1 << 17, (1 << 18) - 1] {
                let mut storage = vec![0; BuddyAllocator::words_needed(frames)];
                let start = PhysAddr::new(base * FRAME_SIZE);
                assert!(
                    BuddyAllocator::new(&mut storage, start, frames).is_ok(),
                    "{frames} {base}"
                );
            }
        }
    }

    #[test]
    fn metadata_costs_about_two_bits_per_frame() {
        let frames = 1 << 20;
        let bits = BuddyAllocator::words_needed(frames) * 64;
        let per_frame_x100 = bits * 100 / frames;
        assert!((200..=210).contains(&per_frame_x100), "{per_frame_x100}");
    }

    /// Resultado final canônico: tudo devolvido tem de dar o mesmo bitmap de quem só liberou.
    #[test]
    fn random_operations_never_overlap_and_everything_comes_back() {
        let (base, frames) = (0x1237, 40_000);
        let mut storage = Vec::new();
        let mut buddy = released(&mut storage, base, frames);
        let mut reference_storage = Vec::new();
        let reference = released(&mut reference_storage, base, frames);
        let canonical = reference.store.to_vec();

        let mut rng = XorShift::new(0x9e37_79b9_7f4a_7c15);
        let mut live: BTreeMap<usize, usize> = BTreeMap::new();
        let mut held = 0;
        for step in 0..400_000 {
            let allocate = live.is_empty() || (rng.below(100) < 55 && held < frames * 9 / 10);
            if allocate {
                let (count, run) = match rng.below(10) {
                    0 => (1 + rng.below(40), true),
                    1..=3 => (1 << rng.below(7), false),
                    _ => (1, false),
                };
                let block = if run {
                    buddy.allocate_contiguous(count)
                } else {
                    buddy.allocate_order(count.trailing_zeros() as usize)
                };
                let Some(block) = block else { continue };
                let start = number(block);
                if !run {
                    assert_eq!(start % count, 0, "passo {step}: desalinhado");
                }
                assert!(
                    start >= base && start + count <= base + frames,
                    "passo {step}: fora"
                );
                if let Some((&prev, &len)) = live.range(..=start).next_back() {
                    assert!(prev + len <= start, "passo {step}: sobrepõe o anterior");
                }
                if let Some((&next, _)) = live.range(start..).next() {
                    assert!(start + count <= next, "passo {step}: sobrepõe o seguinte");
                }
                live.insert(start, count);
                held += count;
            } else {
                let pick = rng.below(live.len());
                let (&start, &count) = live.iter().nth(pick).unwrap();
                live.remove(&start);
                held -= count;
                assert_eq!(
                    buddy.deallocate_contiguous(at(start), count),
                    Ok(()),
                    "passo {step}"
                );
            }
            assert_eq!(buddy.free_frames(), frames - held, "passo {step}");
        }
        for (&start, &count) in &live {
            assert_eq!(buddy.deallocate_contiguous(at(start), count), Ok(()));
        }
        assert_eq!(buddy.free_frames(), frames);
        assert_eq!(buddy.store, canonical.as_slice());
    }
}
