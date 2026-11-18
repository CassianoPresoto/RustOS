//! ChaCha20 do RFC 8439: bloco de 64 bytes e keystream. Sem estado entre chamadas.
//! O contador é `u32` e nunca dá a volta: passar de 2^32 blocos é erro, não repetição.

use crate::wipe::wipe;

/// Tamanho da chave em bytes.
pub const KEY_LEN: usize = 32;
/// Tamanho do nonce em bytes.
pub const NONCE_LEN: usize = 12;
/// Tamanho de um bloco de keystream em bytes.
pub const BLOCK_LEN: usize = 64;

/// "expand 32-byte k" em quatro palavras little-endian (RFC 8439, seção 2.3).
const CONSTANTS: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];

/// O pedido passaria do último contador de bloco (`u32::MAX`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CounterOverflow;

fn word_at(bytes: &[u8], index: usize) -> u32 {
    let at = index * 4;
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// As 16 palavras do estado, ou do estado depois das rodadas. Carrega a chave: não é `Copy`
/// nem `Clone` (cada cópia seria mais uma a zerar) e o `Drop` a zera.
struct Words([u32; 16]);

impl Words {
    const fn zero() -> Self {
        Self([0; 16])
    }

    /// Monta o estado inicial no lugar: devolver `Words` por valor deixaria uma cópia da
    /// chave no quadro de quem o monta.
    fn load(&mut self, key: &[u8; KEY_LEN], counter: u32, nonce: &[u8; NONCE_LEN]) {
        self.0[..4].copy_from_slice(&CONSTANTS);
        for i in 0..8 {
            self.0[4 + i] = word_at(key, i);
        }
        self.0[12] = counter;
        for i in 0..3 {
            self.0[13 + i] = word_at(nonce, i);
        }
    }
}

impl Drop for Words {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

#[inline]
#[allow(
    clippy::many_single_char_names,
    reason = "a, b, c, d são os nomes do RFC 8439"
)]
fn quarter_round(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(7);
}

/// 20 rodadas sobre `state` e a soma final com ele (RFC 8439, seção 2.3), em `out`.
/// O trabalho é feito em `out`, que é de quem chama e é zerado por ele: sem cópia local.
fn permute(state: &Words, out: &mut Words) {
    let x = &mut out.0;
    x.copy_from_slice(&state.0);
    for _ in 0..10 {
        quarter_round(x, 0, 4, 8, 12);
        quarter_round(x, 1, 5, 9, 13);
        quarter_round(x, 2, 6, 10, 14);
        quarter_round(x, 3, 7, 11, 15);
        quarter_round(x, 0, 5, 10, 15);
        quarter_round(x, 1, 6, 11, 12);
        quarter_round(x, 2, 7, 8, 13);
        quarter_round(x, 3, 4, 9, 14);
    }
    for (word, start) in x.iter_mut().zip(&state.0) {
        *word = word.wrapping_add(*start);
    }
}

/// O bloco de 64 bytes da função ChaCha20 (RFC 8439, seção 2.3).
pub fn block(key: &[u8; KEY_LEN], counter: u32, nonce: &[u8; NONCE_LEN]) -> [u8; BLOCK_LEN] {
    let mut out = [0_u8; BLOCK_LEN];
    block_into(key, counter, nonce, &mut out);
    out
}

/// `block` escrevendo em `out`, que é de quem chama: o `Rng` o zera, sem cópia de retorno.
pub(crate) fn block_into(
    key: &[u8; KEY_LEN],
    counter: u32,
    nonce: &[u8; NONCE_LEN],
    out: &mut [u8; BLOCK_LEN],
) {
    let mut state = Words::zero();
    state.load(key, counter, nonce);
    let mut words = Words::zero();
    permute(&state, &mut words);
    for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(&words.0) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
}

fn blocks_fit(counter: u32, len: usize) -> bool {
    let blocks = len.div_ceil(BLOCK_LEN) as u64;
    u64::from(counter) + blocks <= 1 << 32
}

fn apply<const XOR: bool>(state: &mut Words, data: &mut [u8]) {
    let mut words = Words::zero();
    for chunk in data.chunks_mut(BLOCK_LEN) {
        permute(state, &mut words);
        state.0[12] = state.0[12].wrapping_add(1);
        for (i, piece) in chunk.chunks_mut(4).enumerate() {
            let ks = words.0[i].to_le_bytes();
            for (byte, k) in piece.iter_mut().zip(ks) {
                *byte = if XOR { *byte ^ k } else { k };
            }
        }
    }
}

/// Faz XOR de `data` com o keystream que começa no bloco `counter` (RFC 8439,
/// seção 2.4). Cifrar e decifrar são a mesma chamada. Em erro, `data` não muda.
pub fn xor_keystream(
    key: &[u8; KEY_LEN],
    counter: u32,
    nonce: &[u8; NONCE_LEN],
    data: &mut [u8],
) -> Result<(), CounterOverflow> {
    if !blocks_fit(counter, data.len()) {
        return Err(CounterOverflow);
    }
    let mut state = Words::zero();
    state.load(key, counter, nonce);
    apply::<true>(&mut state, data);
    Ok(())
}

/// Sobrescreve `out` com o keystream que começa no bloco `counter`.
/// É o `xor_keystream` sobre zeros, sem ler o que havia em `out`.
pub fn keystream(
    key: &[u8; KEY_LEN],
    counter: u32,
    nonce: &[u8; NONCE_LEN],
    out: &mut [u8],
) -> Result<(), CounterOverflow> {
    if !blocks_fit(counter, out.len()) {
        return Err(CounterOverflow);
    }
    let mut state = Words::zero();
    state.load(key, counter, nonce);
    apply::<false>(&mut state, out);
    Ok(())
}

/// `keystream` sem a conferência do contador: quem chama garante `blocks_fit`.
pub(crate) fn fill_unchecked(
    key: &[u8; KEY_LEN],
    counter: u32,
    nonce: &[u8; NONCE_LEN],
    out: &mut [u8],
) {
    debug_assert!(blocks_fit(counter, out.len()));
    let mut state = Words::zero();
    state.load(key, counter, nonce);
    apply::<false>(&mut state, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(
        clippy::many_single_char_names,
        reason = "a, b, c, d são os nomes do RFC 8439"
    )]
    fn quarter(a: u32, b: u32, c: u32, d: u32) -> [u32; 4] {
        let mut s = [0_u32; 16];
        s[..4].copy_from_slice(&[a, b, c, d]);
        quarter_round(&mut s, 0, 1, 2, 3);
        [s[0], s[1], s[2], s[3]]
    }

    #[test]
    fn quarter_round_matches_rfc_8439_section_2_1_1() {
        let out = quarter(0x1111_1111, 0x0102_0304, 0x9b8d_6f43, 0x0123_4567);
        assert_eq!(out, [0xea2a_92f4, 0xcb1c_f8ce, 0x4581_472e, 0x5881_c4bb]);
    }

    #[test]
    fn quarter_round_on_the_state_matches_rfc_8439_section_2_2_1() {
        let mut state = [
            0x8795_31e0,
            0xc5ec_f37d,
            0x5164_61b1,
            0xc9a6_2f8a,
            0x44c2_0ef3,
            0x3390_af7f,
            0xd9fc_690b,
            0x2a5f_714c,
            0x5337_2767,
            0xb00a_5631,
            0x974c_541a,
            0x359e_9963,
            0x5c97_1061,
            0x3d63_1689,
            0x2098_d9d6,
            0x91db_d320,
        ];
        quarter_round(&mut state, 2, 7, 8, 13);
        let expected = [
            0x8795_31e0,
            0xc5ec_f37d,
            0xbdb8_86dc,
            0xc9a6_2f8a,
            0x44c2_0ef3,
            0x3390_af7f,
            0xd9fc_690b,
            0xcfac_afd2,
            0xe46b_ea80,
            0xb00a_5631,
            0x974c_541a,
            0x359e_9963,
            0x5c97_1061,
            0xccc0_7c79,
            0x2098_d9d6,
            0x91db_d320,
        ];
        assert_eq!(state, expected);
    }

    #[test]
    fn counter_just_below_the_limit_is_accepted_and_beyond_it_is_refused() {
        let key = [7_u8; KEY_LEN];
        let nonce = [0_u8; NONCE_LEN];
        let mut last = [0_u8; BLOCK_LEN];
        assert_eq!(xor_keystream(&key, u32::MAX, &nonce, &mut last), Ok(()));
        assert_eq!(last, block(&key, u32::MAX, &nonce));
        let mut two = [0x5a_u8; BLOCK_LEN + 1];
        assert_eq!(
            xor_keystream(&key, u32::MAX, &nonce, &mut two),
            Err(CounterOverflow)
        );
        assert_eq!(two, [0x5a; BLOCK_LEN + 1]);
        assert_eq!(
            keystream(&key, u32::MAX, &nonce, &mut two),
            Err(CounterOverflow)
        );
    }

    #[test]
    fn empty_input_never_overflows() {
        let key = [0_u8; KEY_LEN];
        let nonce = [0_u8; NONCE_LEN];
        assert_eq!(xor_keystream(&key, u32::MAX, &nonce, &mut []), Ok(()));
    }

    #[test]
    fn keystream_equals_xor_keystream_over_zeros_for_every_length() {
        let key: [u8; KEY_LEN] = core::array::from_fn(|i| i.to_le_bytes()[0]);
        let nonce = [3_u8; NONCE_LEN];
        for len in [0, 1, 3, 4, 63, 64, 65, 127, 128, 129, 200] {
            let mut zeros = std::vec![0_u8; len];
            let mut plain = std::vec![0xff_u8; len];
            xor_keystream(&key, 9, &nonce, &mut zeros).unwrap();
            keystream(&key, 9, &nonce, &mut plain).unwrap();
            assert_eq!(zeros, plain, "len {len}");
        }
    }

    #[test]
    fn xor_keystream_twice_restores_the_message() {
        let key = [0x42_u8; KEY_LEN];
        let nonce = [0x24_u8; NONCE_LEN];
        let original: std::vec::Vec<u8> = (0..300_u32).map(|i| (i * 7).to_le_bytes()[0]).collect();
        let mut data = original.clone();
        xor_keystream(&key, 5, &nonce, &mut data).unwrap();
        assert_ne!(data, original);
        xor_keystream(&key, 5, &nonce, &mut data).unwrap();
        assert_eq!(data, original);
    }

    #[test]
    fn the_state_has_a_destructor_that_wipes_it() {
        assert!(core::mem::needs_drop::<Words>());
        let mut state = Words::zero();
        state.load(&[0xA5; KEY_LEN], 7, &[0x5A; NONCE_LEN]);
        assert!(state.0.iter().any(|&word| word != 0));
        wipe(&mut state.0);
        assert_eq!(state.0, [0; 16]);
    }

    #[test]
    fn load_puts_constants_key_counter_and_nonce_in_the_rfc_order() {
        let key: [u8; KEY_LEN] = core::array::from_fn(|i| i.to_le_bytes()[0]);
        let mut state = Words::zero();
        state.load(&key, 1, &[0, 0, 0, 9, 0, 0, 0, 0x4a, 0, 0, 0, 0]);
        assert_eq!(state.0[..4], CONSTANTS);
        assert_eq!(state.0[4], 0x0302_0100);
        assert_eq!(state.0[11], 0x1f1e_1d1c);
        assert_eq!(state.0[12], 1);
        assert_eq!(state.0[13..], [0x0900_0000, 0x4a00_0000, 0]);
    }

    #[test]
    fn block_into_gives_the_same_bytes_as_block_and_overwrites_the_buffer() {
        let key = [3_u8; KEY_LEN];
        let nonce = [4_u8; NONCE_LEN];
        let mut out = [0xFF_u8; BLOCK_LEN];
        block_into(&key, 2, &nonce, &mut out);
        assert_eq!(out, block(&key, 2, &nonce));
    }
}
