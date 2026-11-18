//! O alocador mínimo: um arco de bytes em `.bss`, avançado por um índice atômico, sem liberar.
//! Até a 4c não há como pedir memória ao kernel (`vmar_map`), então o arco é fixo.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Bytes do arco do usuário: 16 KiB, quatro páginas de `.bss` só para quem aloca.
pub const HEAP_BYTES: usize = 16 * 1024;

/// Um alocador de avanço sobre `N` bytes. `dealloc` não devolve nada.
#[derive(Debug)]
pub struct Bump<const N: usize> {
    arena: UnsafeCell<[u8; N]>,
    next: AtomicUsize,
}

// SAFETY: o índice atômico entrega cada faixa a um só chamador; o arco só se acessa por elas.
unsafe impl<const N: usize> Sync for Bump<N> {}

impl<const N: usize> Bump<N> {
    pub const fn new() -> Self {
        Self {
            arena: UnsafeCell::new([0; N]),
            next: AtomicUsize::new(0),
        }
    }
}

impl<const N: usize> Default for Bump<N> {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: `alloc` só entrega faixas dentro do arco, alinhadas e sem sobreposição; `dealloc` não
// faz nada, então nenhuma faixa volta a ser entregue.
unsafe impl<const N: usize> GlobalAlloc for Bump<N> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let base = self.arena.get().cast::<u8>();
        let mut current = self.next.load(Ordering::Relaxed);
        loop {
            let start = (base.addr() + current).next_multiple_of(layout.align()) - base.addr();
            let Some(end) = start.checked_add(layout.size()).filter(|&end| end <= N) else {
                return ptr::null_mut();
            };
            match self.next.compare_exchange_weak(
                current,
                end,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return base.wrapping_add(start),
                Err(seen) => current = seen,
            }
        }
    }

    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}

#[cfg(all(target_os = "none", not(test)))]
#[global_allocator]
static HEAP: Bump<HEAP_BYTES> = Bump::new();

#[cfg(test)]
mod tests {
    use super::*;

    fn take<const N: usize>(heap: &Bump<N>, size: usize, align: usize) -> *mut u8 {
        let layout = Layout::from_size_align(size, align).unwrap();
        // SAFETY: o `GlobalAlloc` só pede que o layout não tenha tamanho zero; o teste do tamanho
        // zero é o único que o faz e o alocador de avanço o aceita.
        unsafe { heap.alloc(layout) }
    }

    #[test]
    fn allocations_are_aligned_inside_the_arena_and_do_not_overlap() {
        let heap = Bump::<64>::new();
        let (a, b) = (take(&heap, 3, 1), take(&heap, 8, 8));
        assert!(!a.is_null() && !b.is_null());
        assert_eq!(b.addr() % 8, 0);
        assert!(b.addr() >= a.addr() + 3);
        assert!(heap.next.load(Ordering::Relaxed) <= 64);
    }

    #[test]
    fn a_request_that_does_not_fit_gets_null_and_leaves_the_arena_alone() {
        let heap = Bump::<16>::new();
        assert!(take(&heap, 17, 1).is_null());
        assert!(!take(&heap, 16, 1).is_null());
        assert!(take(&heap, 1, 1).is_null());
    }

    #[test]
    fn a_big_alignment_is_honoured() {
        let heap = Bump::<8192>::new();
        assert_eq!(take(&heap, 8, 1024).addr() % 1024, 0);
    }

    #[test]
    fn a_zero_size_request_is_not_null() {
        let heap = Bump::<16>::new();
        assert!(!take(&heap, 0, 1).is_null());
    }
}
