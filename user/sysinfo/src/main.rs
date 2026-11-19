//! `sysinfo`: o que um programa de usuário enxerga do sistema sem privilégio algum: o relógio
//! monotônico, o tamanho da página, quantos `PT_LOAD` ele tem e um número do gerador do kernel.

#![no_std]
#![no_main]
#![forbid(unsafe_code)]

use rustos_user_rt::{Env, boot, entry, println, sys};

entry!(main);

fn main(env: &Env) -> i64 {
    if boot::attach_console(env).is_none() {
        return 1;
    }
    let mut bytes = [0_u8; 8];
    let (Ok(now), Ok(())) = (sys::clock_get_monotonic(), sys::random_get(&mut bytes)) else {
        println!("sysinfo: FAILED to read the clock or the random generator");
        return 2;
    };
    println!(
        "sysinfo: up {} ms, page {} B, {} loads, random {:#018x}",
        now / 1_000_000,
        env.auxv().pagesz,
        env.load_segments().unwrap_or(0),
        u64::from_le_bytes(bytes)
    );
    0
}
