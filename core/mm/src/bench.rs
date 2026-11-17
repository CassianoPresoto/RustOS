//! Medição no host, só com `--ignored`: `cargo test --release -p rustos-mm bench -- --ignored
//! --nocapture --test-threads=1`. Inclui o buddy de listas intrusivas, que não está no kernel.

use core::alloc::Layout;
use core::hint::black_box;
use std::time::Instant;

use rustos_hal::PhysAddr;

use crate::buddy::{BuddyAllocator, FrameAllocator, MAX_ORDER};
use crate::cache::FrameCache;
use crate::frame::{BitmapFrameAllocator, FRAME_SIZE, PhysFrame};
use crate::heap::KernelHeap;
use crate::memory::{FrameMemory, fake::FakeMemory};

const BASE: u64 = 0x4000_0000;
const FRAMES: usize = 1 << 16;
const NIL: u64 = u64::MAX;
const ROUNDS: usize = 40;
const BIG: usize = 200_000;
const ORDERS: usize = MAX_ORDER + 1;

/// Forma (b): listas duplamente encadeadas dentro dos blocos livres, mais um bit por bloco
/// para saber se o irmão está livre. Pega sempre a cabeça da lista, não o menor endereço.
struct ListBuddy<'m, M> {
    mem: &'m mut M,
    heads: [u64; ORDERS],
    bits: Vec<Vec<u64>>,
}

impl<'m, M: FrameMemory> ListBuddy<'m, M> {
    fn new(mem: &'m mut M) -> Self {
        let bits = (0..ORDERS)
            .map(|order| vec![0; (FRAMES >> order).div_ceil(64).max(1)])
            .collect();
        let mut buddy = Self {
            mem,
            heads: [NIL; ORDERS],
            bits,
        };
        buddy.push(0, 16);
        buddy
    }

    fn address(frame: usize) -> u64 {
        BASE + frame as u64 * FRAME_SIZE
    }

    fn frame(address: u64) -> usize {
        ((address - BASE) / FRAME_SIZE) as usize
    }

    fn test(&self, frame: usize, order: usize) -> bool {
        let index = frame >> order;
        self.bits[order][index / 64] & (1 << (index % 64)) != 0
    }

    fn flip(&mut self, frame: usize, order: usize) {
        let index = frame >> order;
        self.bits[order][index / 64] ^= 1 << (index % 64);
    }

    fn push(&mut self, frame: usize, order: usize) {
        let node = Self::address(frame);
        let head = self.heads[order];
        self.mem.write_u64(PhysAddr::new(node), head);
        self.mem.write_u64(PhysAddr::new(node + 8), NIL);
        if head != NIL {
            self.mem.write_u64(PhysAddr::new(head + 8), node);
        }
        self.heads[order] = node;
        self.flip(frame, order);
    }

    fn remove(&mut self, frame: usize, order: usize) {
        let node = Self::address(frame);
        let next = self.mem.read_u64(PhysAddr::new(node));
        let prev = self.mem.read_u64(PhysAddr::new(node + 8));
        if prev == NIL {
            self.heads[order] = next;
        } else {
            self.mem.write_u64(PhysAddr::new(prev), next);
        }
        if next != NIL {
            self.mem.write_u64(PhysAddr::new(next + 8), prev);
        }
        self.flip(frame, order);
    }

    fn allocate(&mut self, order: usize) -> Option<usize> {
        let found = (order..ORDERS).find(|&j| self.heads[j] != NIL)?;
        let frame = Self::frame(self.heads[found]);
        self.remove(frame, found);
        for lower in (order..found).rev() {
            self.push(frame + (1 << lower), lower);
        }
        Some(frame)
    }

    fn free(&mut self, mut frame: usize, mut order: usize) -> bool {
        for above in order..ORDERS {
            if self.test(frame & !((1 << above) - 1), above) {
                return false;
            }
        }
        while order < MAX_ORDER {
            let buddy = frame ^ (1 << order);
            if buddy + (1 << order) > FRAMES || !self.test(buddy, order) {
                break;
            }
            self.remove(buddy, order);
            frame &= !(1 << order);
            order += 1;
        }
        self.push(frame, order);
        true
    }
}

fn start(frame: usize) -> PhysFrame {
    PhysFrame::containing(PhysAddr::new(BASE + frame as u64 * FRAME_SIZE))
}

fn number(frame: PhysFrame) -> usize {
    ((frame.start().as_u64() - BASE) / FRAME_SIZE) as usize
}

fn report(name: &str, ops: usize, started: Instant) {
    let tenths = started.elapsed().as_nanos() * 10 / ops as u128;
    println!("{name:<58} {:>6}.{} ns", tenths / 10, tenths % 10);
}

fn bitmap_storage() -> Vec<u64> {
    vec![0; BitmapFrameAllocator::words_needed(FRAMES)]
}

fn bitmap(storage: &mut [u64]) -> BitmapFrameAllocator<'_> {
    let mut bitmap = BitmapFrameAllocator::new(storage, PhysAddr::new(BASE), FRAMES).unwrap();
    bitmap.release_range(PhysAddr::new(BASE), FRAMES as u64 * FRAME_SIZE);
    bitmap
}

fn buddy(storage: &mut Vec<u64>) -> BuddyAllocator<'_> {
    *storage = vec![0; BuddyAllocator::words_needed(FRAMES)];
    let mut buddy = BuddyAllocator::new(storage, PhysAddr::new(BASE), FRAMES).unwrap();
    buddy.release_range(PhysAddr::new(BASE), FRAMES as u64 * FRAME_SIZE);
    buddy
}

#[test]
#[ignore = "medição: rode com --release --ignored"]
#[allow(clippy::too_many_lines, reason = "roteiro de medição, linear")]
fn bench_frame_allocators() {
    const PAIRS: usize = 2_000_000;
    const RING: usize = 4096;
    println!("\n-- frames, região de {FRAMES} frames (256 MiB) --");

    let mut storage = bitmap_storage();
    let mut bm = bitmap(&mut storage);
    let t = Instant::now();
    for _ in 0..PAIRS {
        let frame = bm.allocate().unwrap();
        bm.deallocate(black_box(frame)).unwrap();
    }
    report("bitmap  alloc+free 1 frame, memória virgem", PAIRS, t);
    let mut ring: Vec<_> = (0..RING).map(|_| bm.allocate().unwrap()).collect();
    let t = Instant::now();
    for step in 0..PAIRS {
        let new = bm.allocate().unwrap();
        let old = core::mem::replace(&mut ring[step % RING], new);
        bm.deallocate(black_box(old)).unwrap();
    }
    report("bitmap  alloc+free 1 frame, anel de 4096 vivos", PAIRS, t);

    let mut storage = Vec::new();
    let mut bd = buddy(&mut storage);
    let t = Instant::now();
    for _ in 0..PAIRS {
        let frame = bd.allocate().unwrap();
        bd.deallocate(black_box(frame)).unwrap();
    }
    report(
        "buddy(a) alloc+free 1 frame, memória virgem (cascata)",
        PAIRS,
        t,
    );
    let mut ring: Vec<_> = (0..RING).map(|_| bd.allocate().unwrap()).collect();
    let t = Instant::now();
    for step in 0..PAIRS {
        let new = bd.allocate().unwrap();
        let old = core::mem::replace(&mut ring[step % RING], new);
        bd.deallocate(black_box(old)).unwrap();
    }
    report("buddy(a) alloc+free 1 frame, anel de 4096 vivos", PAIRS, t);

    let mut mem = FakeMemory::new(BASE, FRAMES * 4096);
    let mut lb = ListBuddy::new(&mut mem);
    let t = Instant::now();
    for _ in 0..PAIRS {
        let frame = lb.allocate(0).unwrap();
        assert!(lb.free(black_box(frame), 0));
    }
    report(
        "buddy(b) alloc+free 1 frame, memória virgem (cascata)",
        PAIRS,
        t,
    );
    let mut ring: Vec<_> = (0..RING).map(|_| lb.allocate(0).unwrap()).collect();
    let t = Instant::now();
    for step in 0..PAIRS {
        let new = lb.allocate(0).unwrap();
        let old = core::mem::replace(&mut ring[step % RING], new);
        assert!(lb.free(black_box(old), 0));
    }
    report("buddy(b) alloc+free 1 frame, anel de 4096 vivos", PAIRS, t);

    let mut storage = Vec::new();
    let mut inner = buddy(&mut storage);
    let mut cache = FrameCache::<64>::new();
    let mut ring: Vec<_> = (0..RING)
        .map(|_| cache.allocate(&mut inner).unwrap())
        .collect();
    let t = Instant::now();
    for step in 0..PAIRS {
        let new = cache.allocate(&mut inner).unwrap();
        let old = core::mem::replace(&mut ring[step % RING], new);
        cache.deallocate(black_box(old), &mut inner).unwrap();
    }
    report(
        "buddy(a)+cache64 alloc+free 1 frame, anel de 4096 vivos",
        PAIRS,
        t,
    );
    println!("-- em lote: 32768 alloc e depois 32768 free, por operação --");
    let ops = ROUNDS * 2 * 32768;
    let mut storage = bitmap_storage();
    let mut bm = bitmap(&mut storage);
    let t = Instant::now();
    for _ in 0..ROUNDS {
        let all: Vec<_> = (0..32768).map(|_| bm.allocate().unwrap()).collect();
        for frame in all {
            bm.deallocate(frame).unwrap();
        }
    }
    report("bitmap  lote", ops, t);
    let mut storage = Vec::new();
    let mut bd = buddy(&mut storage);
    let t = Instant::now();
    for _ in 0..ROUNDS {
        let all: Vec<_> = (0..32768).map(|_| bd.allocate().unwrap()).collect();
        for frame in all {
            bd.deallocate(frame).unwrap();
        }
    }
    report("buddy(a) lote", ops, t);
    let mut mem = FakeMemory::new(BASE, FRAMES * 4096);
    let mut lb = ListBuddy::new(&mut mem);
    let t = Instant::now();
    for _ in 0..ROUNDS {
        let all: Vec<_> = (0..32768).map(|_| lb.allocate(0).unwrap()).collect();
        for frame in all {
            assert!(lb.free(frame, 0));
        }
    }
    report("buddy(b) lote", ops, t);
    let mut storage = Vec::new();
    let mut inner = buddy(&mut storage);
    let mut cache = FrameCache::<64>::new();
    let t = Instant::now();
    for _ in 0..ROUNDS {
        let all: Vec<_> = (0..32768)
            .map(|_| cache.allocate(&mut inner).unwrap())
            .collect();
        for frame in all {
            cache.deallocate(frame, &mut inner).unwrap();
        }
    }
    report("buddy(a)+cache64 lote", ops, t);

    println!("-- bloco de 2 MiB (512 frames) com a metade de baixo ocupada --");
    let mut storage = bitmap_storage();
    let mut bm = bitmap(&mut storage);
    let _used: Vec<_> = (0..32768).map(|_| bm.allocate().unwrap()).collect();
    let t = Instant::now();
    for _ in 0..BIG {
        let run = bm.allocate_contiguous(512).unwrap();
        for index in 0..512 {
            bm.deallocate(start(number(run) + index)).unwrap();
        }
    }
    report("bitmap  contiguous(512) + 512 deallocate (par)", BIG, t);
    let mut storage = Vec::new();
    let mut bd = buddy(&mut storage);
    let _used: Vec<_> = (0..32768).map(|_| bd.allocate().unwrap()).collect();
    let t = Instant::now();
    for _ in 0..BIG {
        let run = bd.allocate_order(9).unwrap();
        bd.deallocate_order(black_box(run), 9).unwrap();
    }
    report("buddy(a) order 9 alloc+free (par)", BIG, t);
    let mut mem = FakeMemory::new(BASE, FRAMES * 4096);
    let mut lb = ListBuddy::new(&mut mem);
    let _used: Vec<_> = (0..32768).map(|_| lb.allocate(0).unwrap()).collect();
    let t = Instant::now();
    for _ in 0..BIG {
        let run = lb.allocate(9).unwrap();
        assert!(lb.free(black_box(run), 9));
    }
    report("buddy(b) order 9 alloc+free (par)", BIG, t);
}

#[test]
#[ignore = "medição: rode com --release --ignored"]
fn bench_heap() {
    const PAIRS: usize = 2_000_000;
    println!("\n-- heap: alloc+free com 1000 objetos vivos --");
    for zero in [false, true] {
        let mut storage = Vec::new();
        let mut frames = buddy(&mut storage);
        let mut heap = KernelHeap::new(FakeMemory::new(BASE, FRAMES * 4096), zero);
        for (size, name) in [
            (16, "16 B"),
            (64, "64 B"),
            (1024, "1024 B"),
            (4096, "4096 B (1 página)"),
            (8192, "8192 B (2 páginas)"),
        ] {
            let layout = Layout::from_size_align(size, 8).unwrap();
            let live: Vec<_> = (0..1000)
                .map(|_| heap.allocate(layout, &mut frames).unwrap())
                .collect();
            let t = Instant::now();
            for _ in 0..PAIRS {
                let ptr = heap.allocate(layout, &mut frames).unwrap();
                heap.deallocate(black_box(ptr), layout, &mut frames)
                    .unwrap();
            }
            report(
                &format!("heap {name:<20} zerar ao liberar = {zero}"),
                PAIRS,
                t,
            );
            for ptr in live {
                heap.deallocate(ptr, layout, &mut frames).unwrap();
            }
        }
    }
    println!("-- heap sobre o cache de 64 frames (slab novo e página grande de 1 frame) --");
    let mut storage = Vec::new();
    let mut inner = buddy(&mut storage);
    let mut cache = FrameCache::<64>::new();
    let mut heap = KernelHeap::new(FakeMemory::new(BASE, FRAMES * 4096), false);
    let layout = Layout::from_size_align(4096, 8).unwrap();
    let t = Instant::now();
    for _ in 0..PAIRS {
        let mut view = cache.through(&mut inner);
        let ptr = heap.allocate(layout, &mut view).unwrap();
        heap.deallocate(black_box(ptr), layout, &mut view).unwrap();
    }
    report("heap 4096 B (1 página) via cache64", PAIRS, t);
    let mut view = cache.through(&mut inner);
    let t = Instant::now();
    for _ in 0..PAIRS {
        let frame = view.allocate_contiguous(1).unwrap();
        view.deallocate_contiguous(black_box(frame), 1).unwrap();
    }
    report(
        "cache64 alloc+free 1 frame, pela vista FrameAllocator",
        PAIRS,
        t,
    );
}

#[test]
#[ignore = "medição: rode com --release --ignored"]
fn bench_metadata() {
    println!("\n-- metadado --");
    for frames in [65_152usize, 1 << 20, 1 << 22] {
        let buddy_bytes = BuddyAllocator::words_needed(frames) * 8;
        let bitmap_bytes = BitmapFrameAllocator::words_needed(frames) * 8;
        println!(
            "{frames:>8} frames ({:>5} MiB): bitmap {bitmap_bytes:>8} B ({:>3} bits/frame x100), buddy {buddy_bytes:>8} B ({:>3} bits/frame x100)",
            frames * 4 / 1024,
            bitmap_bytes * 800 / frames,
            buddy_bytes * 800 / frames,
        );
    }
}
