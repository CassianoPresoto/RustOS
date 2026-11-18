//! O `init`: o primeiro programa de usuário. Confere o que o kernel deixou na pilha inicial
//! (auxv, alinhamento, handle de boot), chama `clock_get_monotonic` e `random_get`, e sai com 0.

#![no_std]
#![no_main]
#![forbid(unsafe_code)]

use rustos_user_rt::{Env, entry, entry_address, println, sys};

entry!(main);

/// `PT_LOAD` de um executável com código e dados em páginas separadas: pelo menos dois.
const MIN_LOADS: usize = 2;

/// O que não bateu, na ordem em que o `init` confere; o código de saída é o primeiro.
fn checks(env: &Env) -> [(&'static str, bool); 8] {
    let aux = env.auxv();
    [
        ("argc is 0", env.argc() == 0),
        ("sp is 16-byte aligned", env.initial_sp().is_multiple_of(16)),
        ("boot handle is 0 until 4c", env.boot_handle() == 0),
        ("AT_PAGESZ is 4096", aux.pagesz == 4096),
        ("AT_ENTRY is _start", aux.entry == entry_address()),
        (
            "AT_PHDR points at the loads",
            env.load_segments().is_some_and(|loads| loads >= MIN_LOADS),
        ),
        ("AT_HWCAP is 0", aux.hwcap == 0 && aux.hwcap2 == 0),
        (
            "AT_RANDOM is present",
            aux.has_random && env.random() != &[0; 16],
        ),
    ]
}

fn main(env: &Env) -> i64 {
    let mut code = 0;
    for (number, (what, ok)) in (1_i64..).zip(checks(env)) {
        if !ok {
            println!("init: FAILED {what}");
            if code == 0 {
                code = number;
            }
        }
    }
    let first = sys::clock_get_monotonic();
    let second = sys::clock_get_monotonic();
    let mut bytes = [0_u8; 16];
    let random = sys::random_get(&mut bytes);
    let now = match (first, second, random) {
        (Ok(first), Ok(second), Ok(()))
            if first > 0 && second >= first && &bytes != env.random() =>
        {
            second
        }
        _ => {
            println!("init: FAILED clock or random");
            return if code == 0 { 100 } else { code };
        }
    };
    if code == 0 {
        let aux = env.auxv();
        println!(
            "init: ok (pagesz {}, entry {:#x}, {} loads, clock {now} ns)",
            aux.pagesz,
            aux.entry,
            env.load_segments().unwrap_or(0)
        );
    }
    code
}
