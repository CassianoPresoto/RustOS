//! Heap do kernel: `GlobalAlloc` sobre `KernelHeap`, com os frames pelo mapa direto.
//! Ordem de lock: `HEAP`, depois `FRAMES`. Código que segura `FRAMES` não aloca.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

use rustos_hal::PhysAddr;
use rustos_mm::{FrameMemory, KernelHeap};

use crate::frames::Frames;
use crate::platform;
use crate::sync::IrqLock;

/// Zerar ao liberar fica ligado (SDD 09); o custo medido está no SDD 07.
const ZERO_ON_FREE: bool = true;

/// Faixa que vai ao zerar de `arch/`: múltipla de 64 bytes e de pelo menos `ZERO_FAST_MIN`
/// (abaixo de 512 bytes o `DC ZVA` perde para o `memset`, SDD 07).
const ZERO_FAST_ALIGN: usize = 64;
const ZERO_FAST_MIN: usize = 512;

/// `FrameMemory` pelo mapa direto: `físico + deslocamento`.
#[derive(Debug)]
pub struct DirectMap {
    offset: u64,
}

impl DirectMap {
    /// # Safety
    /// Todo endereço que os métodos recebem está num frame do kernel, alinhado ao acesso,
    /// visível em `físico + offset` como memória Normal e sem outro acesso concorrente.
    pub const unsafe fn new(offset: u64) -> Self {
        Self { offset }
    }

    fn pointer<T>(&self, addr: PhysAddr) -> *mut T {
        let virt = usize::try_from(addr.as_u64() + self.offset).expect("endereço cabe em usize");
        ptr::with_exposed_provenance_mut(virt)
    }
}

impl FrameMemory for DirectMap {
    fn read_u64(&self, addr: PhysAddr) -> u64 {
        // SAFETY: o endereço cumpre o contrato de `new`; a palavra é alinhada a 8.
        unsafe { self.pointer::<u64>(addr).read() }
    }

    fn write_u64(&mut self, addr: PhysAddr, value: u64) {
        // SAFETY: o endereço cumpre o contrato de `new`; a palavra é alinhada a 8.
        unsafe { self.pointer::<u64>(addr).write(value) };
    }

    fn zero(&mut self, addr: PhysAddr, len: usize) {
        let ptr = self.pointer::<u8>(addr);
        if len >= ZERO_FAST_MIN && (ptr.addr() | len).is_multiple_of(ZERO_FAST_ALIGN) {
            // SAFETY: faixa de frames do kernel (contrato de `new`), em memória Normal, múltipla de 64.
            unsafe { platform::zero_aligned(ptr, len) };
        } else {
            // SAFETY: `[addr, addr + len)` está dentro de frames do kernel, pelo contrato de `new`.
            unsafe { ptr.write_bytes(0, len) };
        }
    }
}

struct State {
    heap: KernelHeap<DirectMap>,
    frames: Frames,
    offset: u64,
    used_bytes: u64,
}

static HEAP: IrqLock<Option<State>> = IrqLock::new(None);

/// Liga o heap aos frames. Uma vez só, antes da primeira alocação.
///
/// # Safety
/// Todo frame que `frames` entrega é acessível em `físico + direct_map_offset`.
pub unsafe fn install(frames: Frames, direct_map_offset: u64) {
    // SAFETY: o contrato desta função é o de `DirectMap::new`.
    let memory = unsafe { DirectMap::new(direct_map_offset) };
    let mut slot = HEAP.lock();
    assert!(slot.is_none(), "heap: instalado duas vezes");
    *slot = Some(State {
        heap: KernelHeap::new(memory, ZERO_ON_FREE),
        frames,
        offset: direct_map_offset,
        used_bytes: 0,
    });
}

/// Páginas seguradas do alocador de frames e bytes pedidos e ainda não devolvidos.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub pages: u64,
    pub used_bytes: u64,
}

pub fn stats() -> Stats {
    HEAP.lock()
        .as_ref()
        .map_or_else(Stats::default, |state| Stats {
            pages: state.heap.pages_held() as u64,
            used_bytes: state.used_bytes,
        })
}

#[derive(Debug)]
struct KernelAllocator;

#[global_allocator]
static ALLOCATOR: KernelAllocator = KernelAllocator;

// SAFETY: `alloc` devolve memória exclusiva do pedido ou nula, e `dealloc` só recebe o
// que `alloc` entregou, com o mesmo `Layout`; tudo sob o lock `HEAP`.
unsafe impl GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut slot = HEAP.lock();
        let Some(state) = slot.as_mut() else {
            return ptr::null_mut();
        };
        let Some(phys) = state.heap.allocate(layout, &mut state.frames) else {
            return ptr::null_mut();
        };
        state.used_bytes += layout.size() as u64;
        let virt = usize::try_from(phys.as_u64() + state.offset).expect("endereço cabe em usize");
        ptr::with_exposed_provenance_mut(virt)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let mut slot = HEAP.lock();
        let Some(state) = slot.as_mut() else {
            panic!("heap: dealloc de {ptr:p} sem heap instalado");
        };
        let phys = PhysAddr::new(ptr.expose_provenance() as u64 - state.offset);
        let result = state.heap.deallocate(phys, layout, &mut state.frames);
        state.used_bytes = state.used_bytes.saturating_sub(layout.size() as u64);
        drop(slot);
        if let Err(error) = result {
            panic!("heap: dealloc de {ptr:p} com {layout:?} falhou: {error:?}");
        }
    }
}

/// Lê uma palavra de um frame já devolvido, para conferir que ele voltou zerado.
fn peek_word(phys: PhysAddr) -> Option<u64> {
    let slot = HEAP.lock();
    let state = slot.as_ref()?;
    // SAFETY: o frame é memória utilizável e o mapa direto a cobre (contrato de `install`).
    let map = unsafe { DirectMap::new(state.offset) };
    Some(map.read_u64(phys))
}

#[repr(align(64))]
struct Line([u8; 64]);

#[repr(align(4096))]
struct Page([u8; 4096]);

#[repr(align(4096))]
struct Page3([u8; 3 * 4096]);

/// Pedidos de vários tamanhos (as sete classes de slab, as bordas e blocos de páginas),
/// conteúdo conferido, e a contabilidade de volta ao ponto de partida.
pub fn self_test() -> bool {
    let baseline = stats().used_bytes;
    let ok = small_and_large_vectors_keep_their_bytes()
        && boxes_respect_alignment()
        && growing_vector_keeps_every_element()
        && stats().used_bytes == baseline;
    ok && freed_pages_come_back_zeroed() && fast_zero_stays_inside_its_range()
}

fn pattern(size: usize, index: usize) -> u8 {
    let low = |value: usize| value.to_le_bytes()[0];
    low(index).wrapping_mul(31).wrapping_add(low(size))
}

fn small_and_large_vectors_keep_their_bytes() -> bool {
    const SIZES: [usize; 15] = [
        1, 7, 16, 17, 100, 512, 1000, 1024, 1025, 2048, 4095, 4096, 4097, 10_000, 70_000,
    ];
    let mut live: Vec<Vec<u8>> = Vec::new();
    for size in SIZES {
        live.push((0..size).map(|index| pattern(size, index)).collect());
    }
    let intact = live.iter().zip(SIZES).all(|(bytes, size)| {
        bytes
            .iter()
            .enumerate()
            .all(|(i, &b)| b == pattern(size, i))
    });
    intact && stats().used_bytes >= SIZES.iter().sum::<usize>() as u64
}

fn boxes_respect_alignment() -> bool {
    let word = Box::new(0x5275_7374_4f53_2121_u64);
    let table = Box::new([7_u32; 300]);
    let line = Box::new(Line([9; 64]));
    let page = Box::new(Page([5; 4096]));
    *word == 0x5275_7374_4f53_2121
        && table.iter().all(|&value| value == 7)
        && ptr::from_ref(&*line).addr().is_multiple_of(64)
        && line.0.iter().all(|&byte| byte == 9)
        && ptr::from_ref(&*page).addr().is_multiple_of(4096)
        && page.0.iter().all(|&byte| byte == 5)
}

fn growing_vector_keeps_every_element() -> bool {
    let mut numbers: Vec<u32> = Vec::new();
    for value in 0..10_000 {
        numbers.push(value);
    }
    (0..10_000).eq(numbers.iter().copied())
}

/// Bloco de duas páginas: volta inteiro ao buddy e cada palavra dele sai zerada.
fn freed_pages_come_back_zeroed() -> bool {
    let Some((frames, offset)) = HEAP
        .lock()
        .as_ref()
        .map(|state| (state.frames, state.offset))
    else {
        return false;
    };
    let before = frames.free_frames();
    let block = alloc::vec![0xAA_u8; 8192];
    let phys = PhysAddr::new(block.as_ptr().expose_provenance() as u64 - offset);
    let held = block.iter().all(|&byte| byte == 0xAA);
    drop(block);
    let words = [0, 8, 4088, 4096, 8184];
    let zeroed = words
        .iter()
        .all(|&at| peek_word(PhysAddr::new(phys.as_u64() + at)) == Some(0));
    held && zeroed && frames.free_frames() == before
}

/// Zerar pelo caminho de `arch/` mexe só na faixa pedida, com começos e tamanhos que não
/// alinham ao bloco do `DC ZVA`: os vizinhos seguem intactos.
fn fast_zero_stays_inside_its_range() -> bool {
    const LEN: usize = 3 * 4096;
    let Some(offset) = HEAP.lock().as_ref().map(|state| state.offset) else {
        return false;
    };
    let mut area = Box::new(Page3([0; LEN]));
    let base = ptr::from_mut(&mut *area).expose_provenance() as u64 - offset;
    // SAFETY: `area` é memória do heap, dentro do mapa direto, só deste teste.
    let mut map = unsafe { DirectMap::new(offset) };
    let cases = [
        (0, 512),
        (64, 512),
        (192, 1024),
        (448, 576),
        (4096 - 64, 576),
        (1024, 4096),
        (320, 8192 + 64),
        (0, LEN),
    ];
    cases.iter().all(|&(start, len)| {
        area.0.fill(0xAA);
        map.zero(PhysAddr::new(base + start as u64), len);
        area.0.iter().enumerate().all(|(at, &byte)| {
            byte == if (start..start + len).contains(&at) {
                0
            } else {
                0xAA
            }
        })
    })
}

/// Custo de zerar, medido no kernel com o relógio da fase 1 (SDD 07).
#[derive(Clone, Copy, Debug)]
pub struct ZeroBench {
    /// Todos em décimos de ns.
    pub kib: u64,
    pub page: u64,
    pub cold_page: u64,
    pub pair: u64,
}

const ZERO_BENCH_ROUNDS: u64 = 4_000;
const ZERO_BENCH_COLD_PAGES: usize = 256;

/// Melhor de três rodadas de `ZERO_BENCH_ROUNDS` repetições de `step`, em ns por repetição.
fn best_ns(mut step: impl FnMut(u64)) -> Option<u64> {
    let mut best = u64::MAX;
    for _ in 0..3 {
        let start = platform::now_ns()?;
        for round in 0..ZERO_BENCH_ROUNDS {
            step(round);
        }
        let spent = platform::now_ns()?.saturating_sub(start);
        best = best.min(spent * 10 / ZERO_BENCH_ROUNDS);
    }
    Some(best)
}

/// Zerar 1 KiB e 4 KiB quentes, 4 KiB numa faixa de 1 MiB, e um par de 4 KiB do heap
/// com a política ligada. Os tempos saem em décimos de ns. `None` sem relógio ou heap.
pub fn zero_benchmark() -> Option<ZeroBench> {
    let offset = HEAP.lock().as_ref()?.offset;
    let area = alloc::vec![0xAA_u8; ZERO_BENCH_COLD_PAGES * 4096 + 4096];
    let base = area.as_ptr().expose_provenance().next_multiple_of(4096) as u64 - offset;
    // SAFETY: a faixa vem do heap, dentro de frames do kernel e do mapa direto, só deste teste.
    let mut map = unsafe { DirectMap::new(offset) };
    let mut zero = |at: u64, len: usize| map.zero(PhysAddr::new(at), len);
    let kib = best_ns(|_| zero(core::hint::black_box(base), 1024))?;
    let page = best_ns(|_| zero(core::hint::black_box(base), 4096))?;
    let cold_page = best_ns(|round| {
        let page = round % ZERO_BENCH_COLD_PAGES as u64;
        zero(core::hint::black_box(base + page * 4096), 4096);
    })?;
    let pair = best_ns(|_| {
        let page = Vec::<u8>::with_capacity(4096);
        core::hint::black_box(&page);
    })?;
    drop(area);
    Some(ZeroBench {
        kib,
        page,
        cold_page,
        pair,
    })
}
