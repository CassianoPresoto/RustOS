//! Programa de teste da paginação sob demanda (fatia 4c do SDD 17): desce a pilha por nove páginas
//! que ninguém mapeou, lê antes de escrever num `.bss` de 32 MiB (a página zero), escreve sobre a
//! página zero e no `.bss` virgem, confere que tudo o que não foi escrito segue zero e mede, por
//! página, os três caminhos da falta.

#![no_std]
#![no_main]
#![forbid(unsafe_code)]

use core::hint::black_box;
use core::sync::atomic::{AtomicU8, Ordering};

use rustos_user_rt::{Env, entry, println, sys};

entry!(main);

const PAGE: usize = 4096;
/// Páginas de cada laço: 16 MiB.
const PAGES: usize = 4096;
const BYTES: usize = PAGES * PAGE;

/// Só escrito pelo laço do primeiro toque.
static FIRST: [AtomicU8; BYTES] = [const { AtomicU8::new(0) }; BYTES];
/// Lido (página zero) e depois escrito (cópia da página zero).
static SECOND: [AtomicU8; BYTES] = [const { AtomicU8::new(0) }; BYTES];

/// Desce `levels` quadros de 3000 bytes, cada um com o próprio marcador, e confere na volta que
/// nenhum foi pisado: as páginas de baixo da pilha nascem na falta.
fn deep(levels: u8) -> bool {
    let mut frame = [levels; 3000];
    black_box(&mut frame);
    let below = levels == 0 || deep(levels - 1);
    black_box(&frame).iter().all(|&byte| byte == levels) && below
}

/// Nanossegundos de `body` por página, em décimos.
fn timed(body: impl FnOnce() -> bool) -> Option<(bool, u64)> {
    let start = sys::clock_get_monotonic().ok()?;
    let ok = body();
    let end = sys::clock_get_monotonic().ok()?;
    Some((ok, end.saturating_sub(start) * 10 / PAGES as u64))
}

fn main(_: &Env) -> i64 {
    let stack = deep(11);
    let first = timed(|| {
        (0..PAGES).all(|page| {
            FIRST[page * PAGE + page % PAGE].store(1, Ordering::Relaxed);
            true
        })
    });
    let read = timed(|| {
        (0..PAGES).all(|page| black_box(&SECOND[page * PAGE]).load(Ordering::Relaxed) == 0)
    });
    let copy = timed(|| {
        (0..PAGES).all(|page| {
            SECOND[page * PAGE].store(2, Ordering::Relaxed);
            true
        })
    });
    let (Some((first_ok, first_ns)), Some((read_ok, read_ns)), Some((copy_ok, copy_ns))) =
        (first, read, copy)
    else {
        return 100;
    };
    let kept = (0..PAGES).all(|page| {
        let (a, b) = (page * PAGE, page * PAGE + PAGE - 1);
        FIRST[a + page % PAGE].load(Ordering::Relaxed) == 1
            && FIRST[b].load(Ordering::Relaxed) == u8::from(page % PAGE == PAGE - 1)
            && SECOND[a].load(Ordering::Relaxed) == 2
            && SECOND[b].load(Ordering::Relaxed) == 0
    });
    let checks = [stack, first_ok, read_ok, copy_ok, kept];
    println!(
        "lazy: {} ({PAGES} pages per loop: first write {}.{} ns, zero-page read {}.{} ns, zero-page copy {}.{} ns per page)",
        if checks.iter().all(|&ok| ok) {
            "ok"
        } else {
            "FAILED"
        },
        first_ns / 10,
        first_ns % 10,
        read_ns / 10,
        read_ns % 10,
        copy_ns / 10,
        copy_ns % 10
    );
    checks.iter().position(|&ok| !ok).map_or(0, |index| {
        i64::try_from(index).map_or(100, |index| index + 1)
    })
}
