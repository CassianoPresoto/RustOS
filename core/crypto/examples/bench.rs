//! Benchmark no host: `cargo run --release -p rustos-crypto --example bench`.
//! Mede ChaCha20, BLAKE2b e o gerador; o melhor de várias rodadas, sem SIMD.

use core::hint::black_box;
use std::time::Instant;

use rustos_crypto::{Rng, blake2b, chacha20};

const ROUNDS: usize = 15;

/// Melhor rodada em nanossegundos. Só inteiros: o bench também roda com FP e SIMD
/// desligados, que é o que os alvos bare-metal têm.
fn best_nanos(mut work: impl FnMut()) -> u128 {
    let mut best = u128::MAX;
    for _ in 0..ROUNDS {
        let start = Instant::now();
        work();
        best = best.min(start.elapsed().as_nanos());
    }
    best
}

fn mb_per_s(bytes: usize, nanos: u128) -> u128 {
    bytes as u128 * 1000 / nanos
}

fn main() {
    let size = 4 << 20;
    let mut buf = vec![0_u8; size];
    let key = [7_u8; 32];
    let nonce = [9_u8; 12];

    let t = best_nanos(|| {
        chacha20::xor_keystream(black_box(&key), 0, &nonce, black_box(&mut buf)).unwrap();
    });
    println!("chacha20 xor_keystream (4 MiB): {} MB/s", mb_per_s(size, t));

    let t = best_nanos(|| {
        for _ in 0..(1 << 16) {
            black_box(chacha20::block(black_box(&key), 1, &nonce));
        }
    });
    println!("chacha20 block (64 B):          {} ns/bloco", t / 65536);

    let mut out = [0_u8; 64];
    let t = best_nanos(|| {
        blake2b::hash(&mut out, &[], black_box(&buf)).unwrap();
    });
    println!("blake2b-512 (4 MiB):            {} MB/s", mb_per_s(size, t));

    let short = [0x5a_u8; 32];
    let mut out32 = [0_u8; 32];
    let t = best_nanos(|| {
        for _ in 0..(1 << 16) {
            blake2b::hash(&mut out32, &key, black_box(&short)).unwrap();
        }
    });
    println!("blake2b-256 keyed (32 B):       {} ns/hash", t / 65536);

    let mut rng = Rng::new(b"bench");
    let t = best_nanos(|| {
        for _ in 0..(1 << 16) {
            black_box(rng.next_u64());
        }
    });
    println!("rng next_u64:                   {} ns", t / 65536);

    let mut page = [0_u8; 4096];
    let t = best_nanos(|| {
        for _ in 0..(1 << 10) {
            rng.fill_bytes(black_box(&mut page));
        }
    });
    println!(
        "rng fill_bytes (4 KiB):         {} MB/s",
        mb_per_s(4096 << 10, t)
    );

    let t = best_nanos(|| {
        for _ in 0..(1 << 14) {
            rng.reseed(black_box(&short));
        }
    });
    println!("rng reseed (32 B):              {} ns", t / 16384);
}
