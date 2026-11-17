//! Porta de memória física: ler, escrever e zerar bytes de frames que já são nossos.
//! O kernel a liga ao mapa direto; os testes, a um `Vec`.

use rustos_hal::PhysAddr;

pub trait FrameMemory {
    /// `addr` é múltiplo de 8 e está dentro de um frame que o chamador possui.
    fn read_u64(&self, addr: PhysAddr) -> u64;
    fn write_u64(&mut self, addr: PhysAddr, value: u64);
    fn zero(&mut self, addr: PhysAddr, len: usize);
}

#[cfg(test)]
pub(crate) mod fake {
    use super::{FrameMemory, PhysAddr};

    /// Memória física falsa: uma faixa de palavras que derruba o teste em acesso fora dela.
    #[derive(Debug)]
    pub struct FakeMemory {
        base: u64,
        words: Vec<u64>,
        pub zeroed_bytes: u64,
    }

    impl FakeMemory {
        pub fn new(base: u64, bytes: usize) -> Self {
            Self {
                base,
                words: vec![0; bytes / 8],
                zeroed_bytes: 0,
            }
        }

        fn slot(&self, addr: PhysAddr) -> usize {
            let offset = addr
                .as_u64()
                .checked_sub(self.base)
                .expect("abaixo da faixa");
            assert_eq!(offset % 8, 0, "acesso desalinhado em {addr:?}");
            let slot = usize::try_from(offset / 8).unwrap();
            assert!(slot < self.words.len(), "acima da faixa em {addr:?}");
            slot
        }
    }

    impl FrameMemory for FakeMemory {
        fn read_u64(&self, addr: PhysAddr) -> u64 {
            self.words[self.slot(addr)]
        }

        fn write_u64(&mut self, addr: PhysAddr, value: u64) {
            let slot = self.slot(addr);
            self.words[slot] = value;
        }

        fn zero(&mut self, addr: PhysAddr, len: usize) {
            let first = self.slot(addr);
            self.words[first..first + len.div_ceil(8)].fill(0);
            self.zeroed_bytes += len as u64;
        }
    }
}
