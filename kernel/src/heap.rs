//! Heap do kernel: `GlobalAlloc` sobre `KernelHeap`, com os frames pelo mapa direto.
//! Ordem de lock: `HEAP`, depois `FRAMES`. Código que segura `FRAMES` não aloca.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

use rustos_hal::PhysAddr;
use rustos_mm::{FrameMemory, KernelHeap};

use crate::addr::Addr;
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

/// Disputas pelo lock do heap desde o boot.
pub fn contentions() -> u32 {
    HEAP.contentions()
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
            panic!(
                "heap: dealloc de {} sem heap instalado",
                Addr(ptr.addr() as u64)
            );
        };
        let phys = PhysAddr::new(ptr.expose_provenance() as u64 - state.offset);
        let result = state.heap.deallocate(phys, layout, &mut state.frames);
        state.used_bytes = state.used_bytes.saturating_sub(layout.size() as u64);
        drop(slot);
        if let Err(error) = result {
            panic!(
                "heap: dealloc de {} com {layout:?} falhou: {error:?}",
                Addr(ptr.addr() as u64)
            );
        }
    }
}

#[cfg(feature = "selftest")]
mod selftest;
#[cfg(feature = "selftest")]
pub use selftest::{self_test, zero_benchmark};
