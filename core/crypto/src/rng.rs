//! Gerador de números aleatórios do kernel: ChaCha20 com "fast key erasure".
//! Puro: a semente (RDSEED, RNDR) e o estado global são do `kernel/`.

use core::fmt;

use crate::{blake2b, chacha20, wipe::wipe};

/// Maior pedido atendido com uma chave só; acima disso a chave troca no meio.
pub const MAX_REQUEST: usize = 1 << 20;

const NONCE: [u8; chacha20::NONCE_LEN] = [0; chacha20::NONCE_LEN];
const HALF: usize = 32;

// O contador do bloco nunca passa de `u32::MAX` dentro de um pedido.
const _: () = assert!(MAX_REQUEST / chacha20::BLOCK_LEN < u32::MAX as usize);

/// Estado: a chave ChaCha20 de 32 bytes. Cada pedido troca a chave antes de
/// devolver; quem lê o estado depois nunca reconstrói a saída anterior.
/// Não é `Clone`: duas cópias do estado dariam a mesma saída duas vezes.
pub struct Rng {
    key: [u8; chacha20::KEY_LEN],
    bytes_since_seed: u64,
}

impl Rng {
    /// Chave de zeros, previsível: só serve para ser semeada no lugar, com `reseed`, de modo
    /// que a chave nunca passe por uma cópia na pilha (`random::init` do kernel).
    pub const fn unseeded() -> Self {
        Self {
            key: [0; chacha20::KEY_LEN],
            bytes_since_seed: 0,
        }
    }

    /// Cria o gerador a partir de `seed`, de qualquer tamanho. Com menos de 256
    /// bits de entropia na semente, o gerador é tão fraco quanto a semente.
    pub fn new(seed: &[u8]) -> Self {
        let mut rng = Self::unseeded();
        rng.reseed(seed);
        rng
    }

    /// Mistura `entropy` na chave: `chave = BLAKE2b-256(chave antiga, entropy)`.
    /// A entrada não apaga o que o gerador já sabia, e zera o contador de bytes.
    pub fn reseed(&mut self, entropy: &[u8]) {
        let mut next = [0_u8; chacha20::KEY_LEN];
        blake2b::keyed_256(&self.key, entropy, &mut next);
        self.key.copy_from_slice(&next);
        wipe(&mut next);
        self.bytes_since_seed = 0;
    }

    /// Enche `out` com bytes aleatórios. Cada chamada troca a chave, então
    /// pedaços não dão a mesma saída que um pedido só; só os primeiros 32
    /// bytes de qualquer pedido não dependem do tamanho dele.
    pub fn fill_bytes(&mut self, out: &mut [u8]) {
        for part in out.chunks_mut(MAX_REQUEST) {
            self.request(part);
        }
    }

    /// Oito bytes do gerador, em little-endian.
    pub fn next_u64(&mut self) -> u64 {
        let mut bytes = [0_u8; 8];
        self.fill_bytes(&mut bytes);
        u64::from_le_bytes(bytes)
    }

    /// Bytes entregues desde o último `new` ou `reseed`, saturando em `u64::MAX`.
    pub fn bytes_since_seed(&self) -> u64 {
        self.bytes_since_seed
    }

    /// O bloco 0 dá a chave nova (primeira metade) e os primeiros 32 bytes de
    /// saída; os blocos seguintes, com a chave velha, dão o resto.
    fn request(&mut self, out: &mut [u8]) {
        let mut first = [0_u8; chacha20::BLOCK_LEN];
        chacha20::block_into(&self.key, 0, &NONCE, &mut first);
        let head = out.len().min(HALF);
        out[..head].copy_from_slice(&first[HALF..HALF + head]);
        if out.len() > HALF {
            chacha20::fill_unchecked(&self.key, 1, &NONCE, &mut out[HALF..]);
        }
        self.key.copy_from_slice(&first[..HALF]);
        wipe(&mut first);
        self.bytes_since_seed = self.bytes_since_seed.saturating_add(out.len() as u64);
    }
}

impl Drop for Rng {
    fn drop(&mut self) {
        wipe(&mut self.key);
    }
}

impl fmt::Debug for Rng {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rng")
            .field("key", &format_args!("<redacted>"))
            .field("bytes_since_seed", &self.bytes_since_seed)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> std::vec::Vec<u8> {
        (0..text.len() / 2)
            .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    fn bytes_of(rng: &mut Rng, len: usize) -> std::vec::Vec<u8> {
        let mut out = std::vec![0_u8; len];
        rng.fill_bytes(&mut out);
        out
    }

    // Os valores vêm de uma implementação independente em Python (ChaCha20 do
    // RFC 8439 em Python puro e `hashlib.blake2b`), que segue o desenho do SDD 09.
    #[test]
    fn output_matches_an_independent_reference_implementation() {
        let mut rng = Rng::new(b"rustos");
        let a = "f45f58f5d8fb8b5fe94b1353fc04d55031f54d845529d457b19b13261925d49c\
                 b95b47fc4ee22db614c0adfb060f926d";
        assert_eq!(bytes_of(&mut rng, 48), hex(a));
        assert_eq!(
            bytes_of(&mut rng, 16),
            hex("6e4c8c020cb39277dfeeff6163a73b92")
        );
        rng.fill_bytes(&mut [0; 100]);
        rng.reseed(b"entropy");
        let d = "c945b7d6a79a8684dc050d635f325055c150b96b6601a132cf85412a239e5e06";
        assert_eq!(bytes_of(&mut rng, 32), hex(d));
    }

    #[test]
    fn an_empty_seed_is_accepted_and_matches_the_reference() {
        let mut rng = Rng::new(b"");
        assert_eq!(bytes_of(&mut rng, 8), hex("31df5b332d7e1430"));
    }

    #[test]
    fn same_seed_gives_the_same_output() {
        let mut a = Rng::new(b"seed");
        let mut b = Rng::new(b"seed");
        assert_eq!(bytes_of(&mut a, 1000), bytes_of(&mut b, 1000));
        assert_eq!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn different_seeds_give_different_output() {
        let mut a = Rng::new(b"seed 1");
        let mut b = Rng::new(b"seed 2");
        assert_ne!(bytes_of(&mut a, 64), bytes_of(&mut b, 64));
    }

    #[test]
    fn one_flipped_bit_in_the_seed_changes_about_half_the_output_bits() {
        let mut seed = [0x33_u8; 32];
        let mut a = Rng::new(&seed);
        seed[17] ^= 1;
        let mut b = Rng::new(&seed);
        let (x, y) = (bytes_of(&mut a, 256), bytes_of(&mut b, 256));
        let differing: u32 = x.iter().zip(&y).map(|(p, q)| (p ^ q).count_ones()).sum();
        assert!(
            (1024 - 150..=1024 + 150).contains(&differing),
            "{differing}"
        );
    }

    #[test]
    fn the_key_changes_after_every_request() {
        let mut rng = Rng::new(b"seed");
        let mut seen = std::vec![rng.key];
        for len in [1, 8, 32, 33, 64, 500] {
            rng.fill_bytes(&mut std::vec![0_u8; len]);
            assert!(!seen.contains(&rng.key), "len {len}");
            seen.push(rng.key);
        }
    }

    #[test]
    fn the_new_key_is_not_part_of_the_output() {
        let mut rng = Rng::new(b"seed");
        let out = bytes_of(&mut rng, 64);
        assert!(!out.windows(32).any(|w| w == rng.key));
    }

    #[test]
    fn an_empty_request_does_not_touch_the_state() {
        let mut rng = Rng::new(b"seed");
        let key = rng.key;
        rng.fill_bytes(&mut []);
        assert_eq!(rng.key, key);
        assert_eq!(rng.bytes_since_seed(), 0);
    }

    #[test]
    fn the_first_32_bytes_of_a_request_do_not_depend_on_its_length() {
        let whole = bytes_of(&mut Rng::new(b"seed"), 300);
        for len in [1, 7, 31, 32, 33, 64, 299] {
            let head = bytes_of(&mut Rng::new(b"seed"), len);
            assert_eq!(head[..len.min(32)], whole[..len.min(32)], "len {len}");
        }
    }

    #[test]
    fn requests_in_pieces_differ_from_one_big_request_because_the_key_changes() {
        let whole = bytes_of(&mut Rng::new(b"seed"), 96);
        let mut rng = Rng::new(b"seed");
        let mut pieces = bytes_of(&mut rng, 32);
        pieces.extend(bytes_of(&mut rng, 64));
        assert_eq!(pieces[..32], whole[..32]);
        assert_ne!(pieces[32..], whole[32..]);
    }

    #[test]
    fn the_same_sequence_of_request_sizes_is_reproducible() {
        let plan = [5, 1, 64, 33, 0, 200, 8];
        let run = |seed: &[u8]| {
            let mut rng = Rng::new(seed);
            plan.iter()
                .flat_map(|&n| bytes_of(&mut rng, n))
                .collect::<std::vec::Vec<_>>()
        };
        assert_eq!(run(b"seed"), run(b"seed"));
        assert_ne!(run(b"seed"), run(b"seee"));
    }

    #[test]
    fn a_request_beyond_the_limit_is_split_in_requests_of_the_limit() {
        let total = 2 * MAX_REQUEST + 5;
        let big = bytes_of(&mut Rng::new(b"seed"), total);
        let mut rng = Rng::new(b"seed");
        let mut parts = bytes_of(&mut rng, MAX_REQUEST);
        parts.extend(bytes_of(&mut rng, MAX_REQUEST));
        parts.extend(bytes_of(&mut rng, 5));
        assert_eq!(big, parts);
    }

    #[test]
    fn next_u64_is_the_first_eight_bytes_little_endian() {
        let mut a = Rng::new(b"seed");
        let mut b = Rng::new(b"seed");
        let bytes = bytes_of(&mut a, 8);
        assert_eq!(b.next_u64(), u64::from_le_bytes(bytes.try_into().unwrap()));
    }

    #[test]
    fn consecutive_next_u64_values_differ() {
        let mut rng = Rng::new(b"seed");
        let values: std::vec::Vec<u64> = (0..1000).map(|_| rng.next_u64()).collect();
        let mut sorted = values.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), values.len());
    }

    #[test]
    fn bytes_since_seed_counts_delivered_bytes_and_reseed_resets_it() {
        let mut rng = Rng::new(b"seed");
        assert_eq!(rng.bytes_since_seed(), 0);
        rng.fill_bytes(&mut [0; 100]);
        let _ = rng.next_u64();
        assert_eq!(rng.bytes_since_seed(), 108);
        rng.reseed(b"more");
        assert_eq!(rng.bytes_since_seed(), 0);
    }

    #[test]
    fn bytes_since_seed_saturates() {
        let mut rng = Rng::new(b"seed");
        rng.bytes_since_seed = u64::MAX - 3;
        rng.fill_bytes(&mut [0; 10]);
        assert_eq!(rng.bytes_since_seed(), u64::MAX);
    }

    #[test]
    fn reseed_changes_the_stream() {
        let mut plain = Rng::new(b"seed");
        let mut mixed = Rng::new(b"seed");
        mixed.reseed(b"entropy");
        assert_ne!(bytes_of(&mut plain, 64), bytes_of(&mut mixed, 64));
    }

    #[test]
    fn reseed_is_deterministic() {
        let mut a = Rng::new(b"seed");
        let mut b = Rng::new(b"seed");
        a.reseed(b"entropy");
        b.reseed(b"entropy");
        assert_eq!(bytes_of(&mut a, 64), bytes_of(&mut b, 64));
    }

    #[test]
    fn reseed_keeps_what_the_state_already_knew() {
        let mut a = Rng::new(b"seed 1");
        let mut b = Rng::new(b"seed 2");
        a.reseed(b"same entropy");
        b.reseed(b"same entropy");
        assert_ne!(a.key, b.key);
    }

    #[test]
    fn reseed_with_empty_entropy_still_changes_the_key() {
        let mut rng = Rng::new(b"seed");
        let before = rng.key;
        rng.reseed(&[]);
        assert_ne!(rng.key, before);
    }

    #[test]
    fn unseeded_has_a_zero_key_and_new_is_unseeded_plus_reseed() {
        let fresh = Rng::unseeded();
        assert_eq!(fresh.key, [0; 32]);
        assert_eq!(fresh.bytes_since_seed(), 0);
        let mut in_place = Rng::unseeded();
        in_place.reseed(b"seed");
        assert_eq!(in_place.key, Rng::new(b"seed").key);
    }

    #[test]
    fn the_state_is_a_32_byte_key_and_an_8_byte_counter() {
        assert_eq!(size_of::<Rng>(), 40);
    }

    #[test]
    fn debug_never_shows_the_key() {
        let mut rng = Rng::new(b"seed");
        rng.fill_bytes(&mut [0; 3]);
        let text = std::format!("{rng:?}");
        assert!(text.contains("<redacted>"));
        assert!(text.contains("bytes_since_seed: 3"));
        for byte in &rng.key[..4] {
            assert!(!text.contains(&std::format!("{byte:#04x}")));
        }
        assert!(!text.contains(&std::format!("{:?}", rng.key)));
    }

    #[test]
    fn output_bits_are_balanced_in_64_kib() {
        let mut rng = Rng::new(b"statistics");
        let out = bytes_of(&mut rng, 1 << 16);
        let ones: u32 = out.iter().map(|b| b.count_ones()).sum();
        // 524 288 bits: média 262 144, desvio 362; a janela é de 6 desvios.
        assert!(ones.abs_diff(262_144) < 6 * 362, "{ones}");
        let mut per_bit = [0_u32; 8];
        for byte in &out {
            for (bit, count) in per_bit.iter_mut().enumerate() {
                *count += u32::from(byte >> bit & 1);
            }
        }
        // 65 536 amostras por bit: média 32 768, desvio 128.
        for (bit, count) in per_bit.iter().enumerate() {
            assert!(count.abs_diff(32_768) < 6 * 128, "bit {bit}: {count}");
        }
    }

    #[test]
    fn every_byte_value_appears_about_equally_in_64_kib() {
        let mut rng = Rng::new(b"statistics");
        let mut counts = [0_u32; 256];
        for byte in bytes_of(&mut rng, 1 << 16) {
            counts[usize::from(byte)] += 1;
        }
        // Média 256, desvio 16; a janela é de 6 desvios.
        for (value, count) in counts.iter().enumerate() {
            assert!(count.abs_diff(256) < 6 * 16, "byte {value}: {count}");
        }
    }
}
