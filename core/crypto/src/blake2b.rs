//! BLAKE2b do RFC 7693: saída de 1 a 64 bytes e chave opcional de até 64 bytes.
//! Sem alocação: o estado é um bloco de 128 bytes na pilha de quem chama.

use core::fmt;

use crate::wipe::wipe;

/// Maior tamanho de saída, em bytes.
pub const MAX_OUTPUT_LEN: usize = 64;
/// Maior tamanho de chave, em bytes.
pub const MAX_KEY_LEN: usize = 64;
/// Tamanho do bloco de compressão, em bytes.
pub const BLOCK_LEN: usize = 128;

const IV: [u64; 8] = [
    0x6a09_e667_f3bc_c908,
    0xbb67_ae85_84ca_a73b,
    0x3c6e_f372_fe94_f82b,
    0xa54f_f53a_5f1d_36f1,
    0x510e_527f_ade6_82d1,
    0x9b05_688c_2b3e_6c1f,
    0x1f83_d9ab_fb41_bd6b,
    0x5be0_cd19_137e_2179,
];

/// Permutações de palavras da mensagem; as rodadas 10 e 11 repetem 0 e 1.
const SIGMA: [[u8; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

/// Parâmetro fora da faixa que o RFC 7693 permite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Saída de 0 bytes, de mais de 64, ou `out` de tamanho diferente do pedido.
    OutputLength,
    /// Chave de mais de 64 bytes.
    KeyLength,
}

/// Estado de um hash em andamento. Guarda a chave no primeiro bloco, por isso
/// o `Drop` zera tudo e o `Debug` não mostra nada.
pub struct Blake2b {
    h: [u64; 8],
    bytes_done: u128,
    buf: [u8; BLOCK_LEN],
    buf_len: usize,
    out_len: usize,
}

impl Blake2b {
    /// Começa um hash de `out_len` bytes (1 a 64), com `key` opcional (0 a 64 bytes).
    pub fn new(out_len: usize, key: &[u8]) -> Result<Self, Error> {
        if out_len == 0 || out_len > MAX_OUTPUT_LEN {
            return Err(Error::OutputLength);
        }
        if key.len() > MAX_KEY_LEN {
            return Err(Error::KeyLength);
        }
        Ok(Self::init(out_len, key))
    }

    /// Como `new`, com `out_len` e `key` já conferidos.
    fn init(out_len: usize, key: &[u8]) -> Self {
        let mut h = IV;
        h[0] ^= 0x0101_0000 ^ ((key.len() as u64) << 8) ^ out_len as u64;
        let mut state = Self {
            h,
            bytes_done: 0,
            buf: [0; BLOCK_LEN],
            buf_len: 0,
            out_len,
        };
        if !key.is_empty() {
            state.buf[..key.len()].copy_from_slice(key);
            state.buf_len = BLOCK_LEN;
        }
        state
    }

    /// Acrescenta `data` à mensagem. O último bloco só é comprimido no `finalize`.
    pub fn update(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            if self.buf_len == BLOCK_LEN {
                self.bytes_done += BLOCK_LEN as u128;
                self.compress(false);
                self.buf_len = 0;
            }
            let take = (BLOCK_LEN - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
        }
    }

    /// Termina e escreve o hash em `out`, que tem de ter exatamente `out_len` bytes.
    pub fn finalize(mut self, out: &mut [u8]) -> Result<(), Error> {
        if out.len() != self.out_len {
            return Err(Error::OutputLength);
        }
        self.finish_into(out);
        Ok(())
    }

    /// `out.len()` é `out_len`: quem chama já conferiu.
    fn finish_into(&mut self, out: &mut [u8]) {
        self.bytes_done += self.buf_len as u128;
        self.buf[self.buf_len..].fill(0);
        self.compress(true);
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = self.h[i / 8].to_le_bytes()[i % 8];
        }
    }

    fn compress(&mut self, last: bool) {
        let mut m = [0_u64; 16];
        for (word, bytes) in m.iter_mut().zip(self.buf.as_chunks::<8>().0) {
            *word = u64::from_le_bytes(*bytes);
        }
        let mut v = [0_u64; 16];
        v[..8].copy_from_slice(&self.h);
        v[8..].copy_from_slice(&IV);
        let (low, high) = split_counter(self.bytes_done);
        v[12] ^= low;
        v[13] ^= high;
        if last {
            v[14] = !v[14];
        }
        for round in 0..12 {
            let s = &SIGMA[round % 10];
            mix(&mut v, [0, 4, 8, 12], m[s[0] as usize], m[s[1] as usize]);
            mix(&mut v, [1, 5, 9, 13], m[s[2] as usize], m[s[3] as usize]);
            mix(&mut v, [2, 6, 10, 14], m[s[4] as usize], m[s[5] as usize]);
            mix(&mut v, [3, 7, 11, 15], m[s[6] as usize], m[s[7] as usize]);
            mix(&mut v, [0, 5, 10, 15], m[s[8] as usize], m[s[9] as usize]);
            mix(&mut v, [1, 6, 11, 12], m[s[10] as usize], m[s[11] as usize]);
            mix(&mut v, [2, 7, 8, 13], m[s[12] as usize], m[s[13] as usize]);
            mix(&mut v, [3, 4, 9, 14], m[s[14] as usize], m[s[15] as usize]);
        }
        for i in 0..8 {
            self.h[i] ^= v[i] ^ v[i + 8];
        }
        wipe(&mut m);
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "as duas metades do contador de 128 bits"
)]
fn split_counter(t: u128) -> (u64, u64) {
    (t as u64, (t >> 64) as u64)
}

/// A função G do RFC 7693 (seção 3.1), com rotações de 32, 24, 16 e 63 bits.
#[inline(always)]
#[allow(
    clippy::inline_always,
    reason = "sem inline o BLAKE2b cai de 1584 para 507 MB/s (Apple M5)"
)]
#[allow(
    clippy::many_single_char_names,
    reason = "a, b, c, d, x, y são os nomes do RFC 7693"
)]
fn mix(v: &mut [u64; 16], [a, b, c, d]: [usize; 4], x: u64, y: u64) {
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
    v[d] = (v[d] ^ v[a]).rotate_right(32);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(24);
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
    v[d] = (v[d] ^ v[a]).rotate_right(16);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(63);
}

impl Drop for Blake2b {
    fn drop(&mut self) {
        wipe(&mut self.h);
        wipe(&mut self.buf);
    }
}

impl fmt::Debug for Blake2b {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Blake2b")
            .field("state", &format_args!("<redacted>"))
            .field("out_len", &self.out_len)
            .finish_non_exhaustive()
    }
}

/// Hash de uma mensagem inteira. O tamanho da saída é `out.len()` (1 a 64).
pub fn hash(out: &mut [u8], key: &[u8], data: &[u8]) -> Result<(), Error> {
    let mut state = Blake2b::new(out.len(), key)?;
    state.update(data);
    state.finalize(out)
}

/// BLAKE2b-256 com chave de 32 bytes: o `reseed` do gerador. Não falha, porque
/// os dois tamanhos estão dentro da faixa.
pub(crate) fn keyed_256(key: &[u8; 32], data: &[u8]) -> [u8; 32] {
    let mut state = Blake2b::init(32, key);
    state.update(data);
    let mut out = [0_u8; 32];
    state.finish_into(&mut out);
    out
}
