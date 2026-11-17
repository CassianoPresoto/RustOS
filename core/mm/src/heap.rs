//! Heap do kernel: slabs de uma página por classe de tamanho, e páginas inteiras do buddy
//! para o que não cabe na maior classe. A API é a que o `GlobalAlloc` embrulha.

use core::alloc::Layout;

use rustos_hal::PhysAddr;

use crate::buddy::FrameAllocator;
use crate::frame::{FRAME_SIZE, FrameError, PhysFrame};
use crate::memory::FrameMemory;

const PAGE: u64 = FRAME_SIZE;
const PAGE_BYTES: usize = 4096;
const _: () = assert!(PAGE_BYTES as u64 == FRAME_SIZE);
const MIN_CLASS_SHIFT: u32 = 4;
/// Maior objeto de slab. Acima dele, o pedido vai em páginas inteiras.
pub const MAX_CLASS: usize = 1024;
pub const CLASSES: usize = (MAX_CLASS.trailing_zeros() - MIN_CLASS_SHIFT + 1) as usize;
/// Cabeçalho no início da página: `next`, `prev`, `free`, `state` (4 palavras).
const HEADER: usize = 32;
const NEXT: u64 = 0;
const PREV: u64 = 8;
const FREE: u64 = 16;
const STATE: u64 = 24;
const NIL: u64 = u64::MAX;
/// Marca de objeto livre na segunda palavra; a primeira é o encadeamento.
const FREE_MAGIC: u64 = 0x5275_7374_4f53_4652;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeapError {
    BadPointer(PhysAddr),
    DoubleFree(PhysAddr),
    Frames(FrameError),
}

/// Bytes de um objeto da classe `class`: 16, 32, …, `MAX_CLASS`.
pub const fn class_size(class: usize) -> usize {
    1 << (class + MIN_CLASS_SHIFT as usize)
}

/// Classe que atende o pedido, ou `None` se ele vai para páginas.
pub fn class_of(layout: Layout) -> Option<usize> {
    let need = layout.size().max(layout.align()).max(1 << MIN_CLASS_SHIFT);
    (need <= MAX_CLASS)
        .then(|| (need.next_power_of_two().trailing_zeros() - MIN_CLASS_SHIFT) as usize)
}

/// Objetos por slab: o cabeçalho ocupa os primeiros `HEADER` bytes, arredondados ao objeto.
pub const fn objects_per_slab(class: usize) -> usize {
    let size = class_size(class);
    (PAGE_BYTES - first_offset(size)) / size
}

const fn first_offset(size: usize) -> usize {
    if size > HEADER { size } else { HEADER }
}

fn pages_for(layout: Layout) -> usize {
    layout
        .size()
        .max(layout.align())
        .div_ceil(PAGE_BYTES)
        .max(1)
}

/// Cada slab é uma página: objetos ainda não entregues vêm de `bump`, os devolvidos formam
/// uma lista livre dentro da própria página. Só slab com vaga fica na lista da classe.
#[derive(Debug)]
pub struct KernelHeap<M> {
    memory: M,
    partial: [u64; CLASSES],
    pages: usize,
    zero_on_free: bool,
}

impl<M: FrameMemory> KernelHeap<M> {
    pub const fn new(memory: M, zero_on_free: bool) -> Self {
        Self {
            memory,
            partial: [NIL; CLASSES],
            pages: 0,
            zero_on_free,
        }
    }

    pub const fn zero_on_free(&self) -> bool {
        self.zero_on_free
    }

    pub const fn set_zero_on_free(&mut self, on: bool) {
        self.zero_on_free = on;
    }

    /// Páginas que o heap segura do buddy: slabs mais blocos grandes.
    pub const fn pages_held(&self) -> usize {
        self.pages
    }

    /// Endereço físico de um bloco de `layout.size()` bytes alinhado a `layout.align()`.
    pub fn allocate<F: FrameAllocator>(
        &mut self,
        layout: Layout,
        frames: &mut F,
    ) -> Option<PhysAddr> {
        let Some(class) = class_of(layout) else {
            let pages = pages_for(layout);
            let first = frames.allocate_contiguous(pages)?;
            self.pages += pages;
            return Some(first.start());
        };
        self.allocate_object(class, frames)
    }

    /// `layout` é o mesmo de `allocate`: dele saem a classe e a quantidade de páginas.
    pub fn deallocate<F: FrameAllocator>(
        &mut self,
        ptr: PhysAddr,
        layout: Layout,
        frames: &mut F,
    ) -> Result<(), HeapError> {
        let Some(class) = class_of(layout) else {
            let pages = pages_for(layout);
            if !ptr.is_aligned(PAGE) {
                return Err(HeapError::BadPointer(ptr));
            }
            frames
                .deallocate_contiguous(PhysFrame::containing(ptr), pages)
                .map_err(HeapError::Frames)?;
            if self.zero_on_free {
                self.memory.zero(ptr, pages * PAGE_BYTES);
            }
            self.pages -= pages;
            return Ok(());
        };
        self.deallocate_object(class, ptr, frames)
    }

    fn allocate_object<F: FrameAllocator>(
        &mut self,
        class: usize,
        frames: &mut F,
    ) -> Option<PhysAddr> {
        let size = class_size(class) as u64;
        let page = match self.partial[class] {
            NIL => self.new_slab(class, frames)?,
            head => head,
        };
        let free = self.read(page + FREE);
        let state = self.read(page + STATE);
        let (used, mut bump) = (state & 0xffff_ffff, state >> 32);
        let (object, next_free) = if free == NIL {
            bump += size;
            (page + bump - size, NIL)
        } else {
            (free, self.read(free + NEXT))
        };
        self.write(page + FREE, next_free);
        self.write(page + STATE, (used + 1) | (bump << 32));
        self.write(object, 0);
        self.write(object + 8, 0);
        if next_free == NIL && bump + size > PAGE {
            self.unlink(class, page);
        }
        Some(PhysAddr::new(object))
    }

    fn deallocate_object<F: FrameAllocator>(
        &mut self,
        class: usize,
        ptr: PhysAddr,
        frames: &mut F,
    ) -> Result<(), HeapError> {
        let (object, size_bytes) = (ptr.as_u64(), class_size(class));
        let size = size_bytes as u64;
        let page = object & !(PAGE - 1);
        let offset = object - page;
        if offset < first_offset(size_bytes) as u64 || !offset.is_multiple_of(size) {
            return Err(HeapError::BadPointer(ptr));
        }
        let state = self.read(page + STATE);
        let (used, bump) = (state & 0xffff_ffff, state >> 32);
        if used == 0 || offset >= bump {
            return Err(HeapError::BadPointer(ptr));
        }
        if self.read(object + 8) == FREE_MAGIC {
            return Err(HeapError::DoubleFree(ptr));
        }
        let free = self.read(page + FREE);
        let was_full = free == NIL && bump + size > PAGE;
        if self.zero_on_free {
            self.memory.zero(ptr, size_bytes);
        }
        self.write(object + NEXT, free);
        self.write(object + 8, FREE_MAGIC);
        self.write(page + FREE, object);
        self.write(page + STATE, (used - 1) | (bump << 32));
        if was_full {
            self.link(class, page);
        }
        if used == 1 && !(self.partial[class] == page && self.read(page + NEXT) == NIL) {
            self.release_slab(class, page, frames)?;
        }
        Ok(())
    }

    fn new_slab<F: FrameAllocator>(&mut self, class: usize, frames: &mut F) -> Option<u64> {
        let page = frames.allocate_contiguous(1)?.start().as_u64();
        self.write(page + FREE, NIL);
        self.write(page + STATE, (first_offset(class_size(class)) as u64) << 32);
        self.link(class, page);
        self.pages += 1;
        Some(page)
    }

    fn release_slab<F: FrameAllocator>(
        &mut self,
        class: usize,
        page: u64,
        frames: &mut F,
    ) -> Result<(), HeapError> {
        self.unlink(class, page);
        let first = PhysAddr::new(page);
        frames
            .deallocate_contiguous(PhysFrame::containing(first), 1)
            .map_err(HeapError::Frames)?;
        if self.zero_on_free {
            self.memory.zero(first, PAGE_BYTES);
        }
        self.pages -= 1;
        Ok(())
    }

    fn link(&mut self, class: usize, page: u64) {
        let head = self.partial[class];
        self.write(page + NEXT, head);
        self.write(page + PREV, NIL);
        if head != NIL {
            self.write(head + PREV, page);
        }
        self.partial[class] = page;
    }

    fn unlink(&mut self, class: usize, page: u64) {
        let (next, prev) = (self.read(page + NEXT), self.read(page + PREV));
        if prev == NIL {
            self.partial[class] = next;
        } else {
            self.write(prev + NEXT, next);
        }
        if next != NIL {
            self.write(next + PREV, prev);
        }
    }

    fn read(&self, addr: u64) -> u64 {
        self.memory.read_u64(PhysAddr::new(addr))
    }

    fn write(&mut self, addr: u64, value: u64) {
        self.memory.write_u64(PhysAddr::new(addr), value);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::buddy::BuddyAllocator;
    use crate::memory::fake::FakeMemory;
    use crate::testing::XorShift;

    const BASE: u64 = 0x4000_0000;
    const REGION: usize = 64 << 20;

    struct Rig {
        storage: Vec<u64>,
    }

    fn rig() -> Rig {
        Rig {
            storage: vec![0; BuddyAllocator::words_needed(REGION / PAGE_BYTES)],
        }
    }

    fn buddy(rig: &mut Rig) -> BuddyAllocator<'_> {
        let mut buddy =
            BuddyAllocator::new(&mut rig.storage, PhysAddr::new(BASE), REGION / 4096).unwrap();
        buddy.release_range(PhysAddr::new(BASE), REGION as u64);
        buddy
    }

    fn heap(zero: bool) -> KernelHeap<FakeMemory> {
        KernelHeap::new(FakeMemory::new(BASE, REGION), zero)
    }

    fn layout(size: usize, align: usize) -> Layout {
        Layout::from_size_align(size, align).unwrap()
    }

    #[test]
    fn classes_are_powers_of_two_from_16_to_1024() {
        assert_eq!(CLASSES, 7);
        assert_eq!((class_size(0), class_size(CLASSES - 1)), (16, 1024));
        assert_eq!(class_of(layout(1, 1)), Some(0));
        assert_eq!(class_of(layout(17, 1)), Some(1));
        assert_eq!(class_of(layout(1024, 8)), Some(6));
        assert_eq!(class_of(layout(1025, 8)), None);
        assert_eq!(class_of(layout(8, 512)), Some(5));
        assert_eq!(class_of(layout(8, 4096)), None);
    }

    #[test]
    fn slab_holds_the_objects_that_fit_after_the_header() {
        let per_slab: Vec<_> = (0..CLASSES).map(objects_per_slab).collect();
        assert_eq!(per_slab, [254, 127, 63, 31, 15, 7, 3]);
    }

    #[test]
    fn every_size_from_1_to_4096_is_aligned_for_every_alignment() {
        let (mut rig, mut heap) = (rig(), heap(false));
        let mut frames = buddy(&mut rig);
        let free = frames.free_frames();
        for shift in 0..=12 {
            let align = 1usize << shift;
            let mut live = Vec::new();
            for size in 1..=4096 {
                let layout = layout(size, align);
                let ptr = heap.allocate(layout, &mut frames).unwrap();
                assert!(
                    ptr.is_aligned(align as u64),
                    "tamanho {size} alinhado a {align}: {ptr:?}"
                );
                live.push((ptr, layout));
            }
            for (ptr, layout) in live {
                assert_eq!(heap.deallocate(ptr, layout, &mut frames), Ok(()));
            }
        }
        assert!(
            free - frames.free_frames() <= CLASSES,
            "só uma página por classe fica de reserva"
        );
    }

    #[test]
    fn freed_object_is_handed_out_again_before_new_memory() {
        let (mut rig, mut heap) = (rig(), heap(false));
        let mut frames = buddy(&mut rig);
        let layout = layout(48, 8);
        let a = heap.allocate(layout, &mut frames).unwrap();
        let b = heap.allocate(layout, &mut frames).unwrap();
        assert_ne!(a, b);
        assert_eq!(heap.deallocate(a, layout, &mut frames), Ok(()));
        assert_eq!(heap.allocate(layout, &mut frames), Some(a));
        assert_eq!(heap.pages_held(), 1);
    }

    #[test]
    fn a_full_slab_leaves_the_list_and_a_new_page_takes_over() {
        let (mut rig, mut heap) = (rig(), heap(false));
        let mut frames = buddy(&mut rig);
        let layout = layout(1024, 8);
        let objects: Vec<_> = (0..4)
            .map(|_| heap.allocate(layout, &mut frames).unwrap())
            .collect();
        assert_eq!(heap.pages_held(), 2);
        let pages: std::collections::HashSet<_> =
            objects.iter().map(|ptr| ptr.as_u64() / 4096).collect();
        assert_eq!(pages.len(), 2);
        assert_eq!(heap.deallocate(objects[0], layout, &mut frames), Ok(()));
        assert_eq!(heap.allocate(layout, &mut frames), Some(objects[0]));
    }

    #[test]
    fn empty_slabs_go_back_to_the_buddy_but_one_stays_as_reserve() {
        let (mut rig, mut heap) = (rig(), heap(false));
        let mut frames = buddy(&mut rig);
        let free = frames.free_frames();
        let layout = layout(100, 8);
        let objects: Vec<_> = (0..500)
            .map(|_| heap.allocate(layout, &mut frames).unwrap())
            .collect();
        assert_eq!(
            free - frames.free_frames(),
            500usize.div_ceil(objects_per_slab(3))
        );
        for ptr in objects {
            assert_eq!(heap.deallocate(ptr, layout, &mut frames), Ok(()));
        }
        assert_eq!((heap.pages_held(), free - frames.free_frames()), (1, 1));
    }

    #[test]
    fn detects_double_free_and_bad_pointers() {
        let (mut rig, mut heap) = (rig(), heap(false));
        let mut frames = buddy(&mut rig);
        let layout = layout(64, 8);
        let a = heap.allocate(layout, &mut frames).unwrap();
        let _keep = heap.allocate(layout, &mut frames).unwrap();
        assert_eq!(heap.deallocate(a, layout, &mut frames), Ok(()));
        assert_eq!(
            heap.deallocate(a, layout, &mut frames),
            Err(HeapError::DoubleFree(a))
        );
        let inside = PhysAddr::new(a.as_u64() + 8);
        assert_eq!(
            heap.deallocate(inside, layout, &mut frames),
            Err(HeapError::BadPointer(inside))
        );
        let header = PhysAddr::new(a.as_u64() & !4095);
        assert_eq!(
            heap.deallocate(header, layout, &mut frames),
            Err(HeapError::BadPointer(header))
        );
        let untouched = PhysAddr::new(a.as_u64() + 2048);
        assert_eq!(
            heap.deallocate(untouched, layout, &mut frames),
            Err(HeapError::BadPointer(untouched))
        );
        let page = PhysAddr::new(BASE + 8192);
        assert_eq!(
            heap.deallocate(page, self::layout(4096, 8), &mut frames),
            Err(HeapError::Frames(FrameError::DoubleFree(page)))
        );
        let odd = PhysAddr::new(BASE + 8192 + 8);
        assert_eq!(
            heap.deallocate(odd, self::layout(4096, 8), &mut frames),
            Err(HeapError::BadPointer(odd))
        );
    }

    #[test]
    fn a_live_object_that_holds_the_free_marker_is_not_a_double_free_after_reuse() {
        let (mut rig, mut heap) = (rig(), heap(false));
        let mut frames = buddy(&mut rig);
        let layout = layout(32, 8);
        let a = heap.allocate(layout, &mut frames).unwrap();
        assert_eq!(heap.deallocate(a, layout, &mut frames), Ok(()));
        let again = heap.allocate(layout, &mut frames).unwrap();
        assert_eq!(heap.deallocate(again, layout, &mut frames), Ok(()));
    }

    #[test]
    fn large_requests_take_exactly_the_pages_they_need() {
        let (mut rig, mut heap) = (rig(), heap(false));
        let mut frames = buddy(&mut rig);
        let free = frames.free_frames();
        let layout = layout(4096 * 3 - 5, 8);
        let ptr = heap.allocate(layout, &mut frames).unwrap();
        assert!(ptr.is_aligned(4096));
        assert_eq!((free - frames.free_frames(), heap.pages_held()), (3, 3));
        assert_eq!(heap.deallocate(ptr, layout, &mut frames), Ok(()));
        assert_eq!((frames.free_frames(), heap.pages_held()), (free, 0));
    }

    #[test]
    fn large_requests_honour_big_alignments() {
        let (mut rig, mut heap) = (rig(), heap(false));
        let mut frames = buddy(&mut rig);
        let layout = layout(100, 1 << 16);
        let ptr = heap.allocate(layout, &mut frames).unwrap();
        assert!(ptr.is_aligned(1 << 16));
        assert_eq!(heap.deallocate(ptr, layout, &mut frames), Ok(()));
    }

    #[test]
    fn refuses_when_the_buddy_is_exhausted() {
        let (mut rig, mut heap) = (rig(), heap(false));
        let mut frames = buddy(&mut rig);
        assert_eq!(heap.allocate(layout(REGION + 1, 8), &mut frames), None);
        while heap.allocate(layout(1 << 20, 8), &mut frames).is_some() {}
        while heap.allocate(layout(4096, 8), &mut frames).is_some() {}
        assert_eq!(heap.allocate(layout(16, 8), &mut frames), None);
    }

    #[test]
    fn zero_on_free_wipes_objects_and_returned_pages() {
        let (mut rig, mut heap) = (rig(), heap(true));
        let mut frames = buddy(&mut rig);
        let layout = layout(256, 8);
        let ptr = heap.allocate(layout, &mut frames).unwrap();
        for word in 2..32 {
            heap.memory
                .write_u64(PhysAddr::new(ptr.as_u64() + word * 8), 0xdead_beef);
        }
        let _keep = heap.allocate(layout, &mut frames).unwrap();
        assert_eq!(heap.deallocate(ptr, layout, &mut frames), Ok(()));
        for word in 2..32 {
            assert_eq!(
                heap.memory.read_u64(PhysAddr::new(ptr.as_u64() + word * 8)),
                0
            );
        }
        assert_eq!(heap.memory.zeroed_bytes, 256);

        let big = self::layout(8192, 8);
        let block = heap.allocate(big, &mut frames).unwrap();
        heap.memory.write_u64(block, 7);
        heap.memory.zeroed_bytes = 0;
        assert_eq!(heap.deallocate(block, big, &mut frames), Ok(()));
        assert_eq!(
            (heap.memory.read_u64(block), heap.memory.zeroed_bytes),
            (0, 8192)
        );
    }

    #[test]
    fn without_the_policy_nothing_is_wiped() {
        let (mut rig, mut heap) = (rig(), heap(false));
        let mut frames = buddy(&mut rig);
        let layout = layout(256, 8);
        let ptr = heap.allocate(layout, &mut frames).unwrap();
        heap.memory
            .write_u64(PhysAddr::new(ptr.as_u64() + 16), 0xdead_beef);
        let _keep = heap.allocate(layout, &mut frames).unwrap();
        assert_eq!(heap.deallocate(ptr, layout, &mut frames), Ok(()));
        assert_eq!(heap.memory.zeroed_bytes, 0);
        assert!(!heap.zero_on_free());
        heap.set_zero_on_free(true);
        assert!(heap.zero_on_free());
    }

    #[test]
    fn allocating_clears_the_words_a_free_object_used_for_its_links() {
        let (mut rig, mut heap) = (rig(), heap(false));
        let mut frames = buddy(&mut rig);
        let layout = layout(64, 8);
        let a = heap.allocate(layout, &mut frames).unwrap();
        let _keep = heap.allocate(layout, &mut frames).unwrap();
        assert_eq!(heap.deallocate(a, layout, &mut frames), Ok(()));
        assert_eq!(heap.allocate(layout, &mut frames), Some(a));
        assert_eq!(
            (
                heap.memory.read_u64(a),
                heap.memory.read_u64(PhysAddr::new(a.as_u64() + 8))
            ),
            (0, 0)
        );
    }

    #[test]
    fn random_allocations_never_overlap_keep_their_contents_and_all_come_back() {
        let (mut rig, mut heap) = (rig(), heap(true));
        let mut frames = buddy(&mut rig);
        let free = frames.free_frames();
        let mut rng = XorShift::new(0x1234_5678_9abc_def1);
        let mut live: BTreeMap<u64, (usize, Layout)> = BTreeMap::new();
        for step in 0..300_000usize {
            if live.is_empty() || rng.below(100) < 52 {
                let size = match rng.below(20) {
                    0 => 1 + rng.below(20_000),
                    1..=4 => 1 + rng.below(2048),
                    _ => 1 + rng.below(128),
                };
                let layout = layout(size, 1 << rng.below(8));
                let Some(ptr) = heap.allocate(layout, &mut frames) else {
                    continue;
                };
                let start = ptr.as_u64();
                assert!(ptr.is_aligned(layout.align() as u64), "passo {step}");
                if let Some((&prev, &(_, old))) = live.range(..=start).next_back() {
                    assert!(
                        prev + old.size() as u64 <= start,
                        "passo {step}: sobrepõe o anterior"
                    );
                }
                if let Some((&next, _)) = live.range(start..).next() {
                    assert!(
                        start + layout.size() as u64 <= next,
                        "passo {step}: sobrepõe o seguinte"
                    );
                }
                heap.memory.write_u64(ptr, step as u64);
                live.insert(start, (step, layout));
            } else {
                let (&start, &(tag, layout)) = live.iter().nth(rng.below(live.len())).unwrap();
                assert_eq!(
                    heap.memory.read_u64(PhysAddr::new(start)),
                    tag as u64,
                    "passo {step}: conteúdo mudou"
                );
                live.remove(&start);
                assert_eq!(
                    heap.deallocate(PhysAddr::new(start), layout, &mut frames),
                    Ok(()),
                    "passo {step}"
                );
            }
        }
        for (&start, &(_, layout)) in &live {
            assert_eq!(
                heap.deallocate(PhysAddr::new(start), layout, &mut frames),
                Ok(())
            );
        }
        assert!(heap.pages_held() <= CLASSES);
        assert_eq!(free - frames.free_frames(), heap.pages_held());
    }
}
