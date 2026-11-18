//! Marca d'água das pilhas do kernel: pintada com um padrão ao mapear, e o pico é até onde o
//! padrão sumiu, varrendo do fundo (SDD 15, "Pico de pilha").

use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering};

use rustos_hal::{PAGE_SIZE, VirtAddr};

/// Nenhum valor que uma pilha escreva de propósito; sem ele o pico sai menor que o real.
const PATTERN: u64 = 0x5253_5441_434b_2121;
const WORD: usize = 8;
const LINE_WORDS: usize = 64 / WORD;

/// Uma pilha mapeada: do topo (fim exclusivo) para baixo, sem a página de guarda.
#[derive(Clone, Copy, Debug)]
pub struct Stack {
    top: usize,
    bytes: usize,
}

impl Stack {
    /// `top` é o que `map_stack` devolveu e `pages` o que ela recebeu.
    pub fn new(top: VirtAddr, pages: u64) -> Self {
        Self {
            top: usize::try_from(top.as_u64()).unwrap_or(0),
            bytes: usize::try_from(pages * PAGE_SIZE).unwrap_or(0),
        }
    }

    pub fn top(self) -> VirtAddr {
        VirtAddr::new(self.top as u64)
    }

    pub fn bytes(self) -> usize {
        self.bytes
    }

    fn words(self) -> usize {
        self.bytes / WORD
    }

    fn word(self, index: usize) -> &'static AtomicU64 {
        let at = self.top - self.bytes + index * WORD;
        // SAFETY: a pilha fica mapeada para sempre, alinhada a 8, e só se acessa com atômico.
        unsafe { AtomicU64::from_ptr(ptr::with_exposed_provenance_mut(at)) }
    }

    /// Cobre a pilha inteira com o padrão. Só de pilha que nenhuma tarefa usa agora.
    pub fn paint(self) {
        for index in 0..self.words() {
            self.word(index).store(PATTERN, Ordering::Relaxed);
        }
    }

    /// O índice da primeira palavra que o padrão não cobre, do fundo para cima. Olha uma palavra
    /// por linha de cache e refina na última linha limpa: o pico tem resolução de 8 bytes.
    fn first_used(self) -> usize {
        let mut line = 0;
        while line < self.words() && self.word(line).load(Ordering::Relaxed) == PATTERN {
            line += LINE_WORDS;
        }
        (line.saturating_sub(LINE_WORDS - 1)..self.words())
            .find(|&index| self.word(index).load(Ordering::Relaxed) != PATTERN)
            .unwrap_or(self.words())
    }

    /// Bytes que a pilha já usou: do topo até a palavra mais funda que o padrão não cobre.
    pub fn peak(self) -> usize {
        self.bytes - self.first_used() * WORD
    }

    /// Devolve o pico e repinta só o que foi usado: a pilha de uma tarefa nova que reusa o slot.
    pub fn repaint(self) -> usize {
        let first = self.first_used();
        for index in first..self.words() {
            self.word(index).store(PATTERN, Ordering::Relaxed);
        }
        self.bytes - first * WORD
    }
}
