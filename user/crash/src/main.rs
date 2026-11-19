//! `crash`: desce a pilha até a página de guarda. A falta de página não se resolve, o kernel mata só
//! este processo e o shell mostra a causa. Em Rust seguro: a recursão é o endereço proibido.

#![no_std]
#![no_main]
#![forbid(unsafe_code)]

use core::hint::black_box;

use rustos_user_rt::{Env, boot, entry, println};

entry!(main);

/// Um quadro de 1 KiB por nível: a pilha de 64 KiB acaba em poucas dezenas de níveis.
fn descend(depth: u64) -> u64 {
    let mut frame = [0_u8; 1024];
    frame[usize::try_from(depth % 1024).unwrap_or(0)] = 1;
    black_box(&mut frame);
    let below = if black_box(depth) == u64::MAX {
        0
    } else {
        descend(depth + 1)
    };
    below + u64::from(black_box(&frame)[0])
}

fn main(env: &Env) -> i64 {
    if boot::attach_console(env).is_none() {
        return 1;
    }
    println!("crash: going down the stack until the guard page stops me");
    let reached = descend(0);
    println!("crash: this line never prints ({reached})");
    0
}
