//! Programa de teste maior que o mínimo (cerca de 2,3 MiB): uma tabela de 1 MiB em `.rodata`, 256 KiB
//! (e 400 bytes) em `.data` e 1 MiB em `.bss`, com leituras e escritas nas pontas e no meio de cada uma. Fecha o item
//! do `code-model=small` do SDD 17 e passa o carregador por segmentos de muitas páginas.

#![no_std]
#![no_main]
#![forbid(unsafe_code)]

use core::hint::black_box;
use core::sync::atomic::{AtomicU32, Ordering};

use rustos_user_rt::{Env, entry, println};

entry!(main);

const RODATA_COUNT: u32 = 256 * 1024;
const DATA_COUNT: u32 = 64 * 1024 + 100;
const BSS_COUNT: u32 = 256 * 1024;
const RODATA_WORDS: usize = RODATA_COUNT as usize;
const DATA_WORDS: usize = DATA_COUNT as usize;
const BSS_WORDS: usize = BSS_COUNT as usize;
const PATTERN: u32 = 0x1234_5678;

static TABLE: [u32; RODATA_WORDS] = [PATTERN; RODATA_WORDS];
static DATA: [AtomicU32; DATA_WORDS] = [const { AtomicU32::new(1) }; DATA_WORDS];
static BSS: [AtomicU32; BSS_WORDS] = [const { AtomicU32::new(0) }; BSS_WORDS];

fn rodata_sum() -> u32 {
    TABLE
        .iter()
        .fold(0_u32, |sum, word| sum.wrapping_add(black_box(*word)))
}

/// Soma `delta` a toda palavra e devolve a soma dos valores novos.
fn bump(words: &[AtomicU32], delta: u32) -> u32 {
    words.iter().fold(0_u32, |sum, word| {
        sum.wrapping_add(word.fetch_add(delta, Ordering::Relaxed).wrapping_add(delta))
    })
}

fn main(_: &Env) -> i64 {
    let rodata = rodata_sum();
    let data = bump(&DATA, 2);
    let bss = bump(&BSS, 3);
    let ok = rodata == PATTERN.wrapping_mul(RODATA_COUNT)
        && data == 3 * DATA_COUNT
        && bss == 3 * BSS_COUNT
        && black_box(&BSS)[BSS_WORDS - 1].load(Ordering::Relaxed) == 3;
    println!("big: rodata {rodata:#x}, data {data:#x}, bss {bss:#x}");
    i64::from(!ok)
}
