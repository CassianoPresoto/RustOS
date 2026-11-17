//! Apoio dos testes: gerador pseudoaleatório determinístico.

/// xorshift64*: o mesmo fluxo em toda máquina, para o teste ser reproduzível.
#[derive(Debug)]
pub struct XorShift(u64);

impl XorShift {
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Valor em `[0, bound)`.
    pub fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() >> 33).unwrap() % bound
    }
}
