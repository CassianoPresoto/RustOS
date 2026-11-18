//! Marca d'água das pilhas do kernel: pintada com um padrão ao mapear, e o pico é até onde o
//! padrão sumiu, varrendo do fundo (SDD 15, "Pico de pilha").

use core::ptr;

use rustos_hal::{PAGE_SIZE, VirtAddr};
use rustos_mm::watermark::first_unpainted;

const PATTERN: u64 = rustos_hal::STACK_PAINT;
const WORD: usize = 8;

/// Uma pilha mapeada: do topo (fim exclusivo) para baixo, sem a página de guarda.
#[derive(Clone, Copy, Debug)]
pub struct Stack {
    top: usize,
    bytes: usize,
}

impl Stack {
    /// # Safety
    /// As `pages` páginas abaixo de `top` ficam mapeadas para sempre como leitura e escrita,
    /// alinhadas a 8, e `pages` não é zero.
    pub unsafe fn new(top: VirtAddr, pages: u64) -> Self {
        Self {
            top: usize::try_from(top.as_u64()).expect("endereço cabe em usize"),
            bytes: usize::try_from(pages * PAGE_SIZE).expect("tamanho cabe em usize"),
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

    fn at(self, index: usize) -> *mut u64 {
        ptr::with_exposed_provenance_mut(self.top - self.bytes + index * WORD)
    }

    fn read(self, index: usize) -> u64 {
        // SAFETY: mapeada para sempre (contrato de `new`). A leitura pode correr com a escrita
        // da thread dona: o valor só alimenta estatística, e uma palavra alinhada não rasga.
        unsafe { ptr::read_volatile(self.at(index)) }
    }

    fn write(self, index: usize) {
        // SAFETY: mapeada para sempre (contrato de `new`) e, pelo contrato de `paint` e
        // `repaint`, sem thread que a use.
        unsafe { ptr::write_volatile(self.at(index), PATTERN) };
    }

    /// Cobre a pilha inteira com o padrão.
    ///
    /// # Safety
    /// Nenhuma thread usa a pilha.
    pub unsafe fn paint(self) {
        for index in 0..self.words() {
            self.write(index);
        }
    }

    fn first_used(self) -> usize {
        first_unpainted(self.words(), PATTERN, |index| self.read(index))
    }

    /// Bytes que a pilha já usou: do topo até a palavra mais funda que o padrão não cobre.
    /// Não vê o que a thread reservou e não escreveu abaixo da palavra mais funda que escreveu.
    pub fn peak(self) -> usize {
        self.bytes - self.first_used() * WORD
    }

    /// Devolve o pico e repinta só o que foi usado: a pilha de uma tarefa nova que reusa o slot.
    ///
    /// # Safety
    /// Nenhuma thread usa a pilha.
    pub unsafe fn repaint(self) -> usize {
        let first = self.first_used();
        for index in first..self.words() {
            self.write(index);
        }
        self.bytes - first * WORD
    }
}
