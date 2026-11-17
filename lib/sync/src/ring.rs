//! Fila de bytes de um produtor e um consumidor, sem lock: a interrupção põe,
//! o laço do shell tira. Capacidade fixa; cheia, o byte novo é descartado.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

/// `N` é potência de dois. `head` só o produtor escreve; `tail`, só o consumidor.
#[derive(Debug)]
pub struct ByteRing<const N: usize> {
    slots: [UnsafeCell<u8>; N],
    head: AtomicUsize,
    tail: AtomicUsize,
}

// SAFETY: cada slot é escrito só pelo produtor antes do `Release` de `head`, e
// lido só pelo consumidor depois do `Acquire` dele (contrato de `push`/`pop`).
unsafe impl<const N: usize> Sync for ByteRing<N> {}

impl<const N: usize> ByteRing<N> {
    pub const fn new() -> Self {
        const { assert!(N.is_power_of_two() && N >= 2) };
        Self {
            slots: [const { UnsafeCell::new(0) }; N],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Põe um byte; `false` se a fila estava cheia.
    ///
    /// # Safety
    /// Um produtor só: nenhuma outra chamada a `push` corre ao mesmo tempo.
    pub unsafe fn push(&self, byte: u8) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) == N {
            return false;
        }
        // SAFETY: o slot está livre (o consumidor já passou dele) e só o produtor escreve aqui.
        unsafe { *self.slots[head % N].get() = byte };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    /// Tira o byte mais antigo.
    ///
    /// # Safety
    /// Um consumidor só: nenhuma outra chamada a `pop` corre ao mesmo tempo.
    pub unsafe fn pop(&self) -> Option<u8> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if head == tail {
            return None;
        }
        // SAFETY: o `Acquire` de `head` torna visível a escrita do produtor neste slot.
        let byte = unsafe { *self.slots[tail % N].get() };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(byte)
    }

    pub fn len(&self) -> usize {
        let tail = self.tail.load(Ordering::Acquire);
        self.head.load(Ordering::Acquire).wrapping_sub(tail)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<const N: usize> Default for ByteRing<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::*;

    /// Nos testes, cada fila tem uma thread produtora e uma consumidora.
    fn push<const N: usize>(ring: &ByteRing<N>, byte: u8) -> bool {
        // SAFETY: o teste que chama é o único produtor desta fila.
        unsafe { ring.push(byte) }
    }

    fn pop<const N: usize>(ring: &ByteRing<N>) -> Option<u8> {
        // SAFETY: o teste que chama é o único consumidor desta fila.
        unsafe { ring.pop() }
    }

    #[test]
    fn keeps_fifo_order_and_reports_empty() {
        let ring = ByteRing::<4>::new();
        assert!(push(&ring, 1));
        assert!(push(&ring, 2));
        assert_eq!(ring.len(), 2);
        assert_eq!(pop(&ring), Some(1));
        assert_eq!(pop(&ring), Some(2));
        assert_eq!(pop(&ring), None);
        assert!(ring.is_empty());
    }

    #[test]
    fn full_ring_drops_the_new_byte() {
        let ring = ByteRing::<2>::new();
        assert!(push(&ring, 1));
        assert!(push(&ring, 2));
        assert!(!push(&ring, 3));
        assert_eq!(pop(&ring), Some(1));
        assert!(push(&ring, 4));
        assert_eq!(pop(&ring), Some(2));
        assert_eq!(pop(&ring), Some(4));
    }

    #[test]
    fn indices_survive_wrapping_past_usize_max() {
        let ring = ByteRing::<4>::new();
        ring.head.store(usize::MAX - 1, Ordering::Relaxed);
        ring.tail.store(usize::MAX - 1, Ordering::Relaxed);
        for byte in 0..4 {
            assert!(push(&ring, byte));
        }
        assert!(!push(&ring, 9));
        for byte in 0..4 {
            assert_eq!(pop(&ring), Some(byte));
        }
    }

    #[test]
    fn one_producer_and_one_consumer_thread_lose_nothing() {
        const TOTAL: usize = 200_000;
        let ring = Arc::new(ByteRing::<64>::new());
        let producer = {
            let ring = Arc::clone(&ring);
            thread::spawn(move || {
                for index in 0..TOTAL {
                    let byte = u8::try_from(index % 251).unwrap();
                    while !push(&ring, byte) {
                        core::hint::spin_loop();
                    }
                }
            })
        };
        for index in 0..TOTAL {
            let expected = u8::try_from(index % 251).unwrap();
            loop {
                if let Some(byte) = pop(&ring) {
                    assert_eq!(byte, expected);
                    break;
                }
                core::hint::spin_loop();
            }
        }
        producer.join().unwrap();
        assert!(ring.is_empty());
    }
}
