//! Programa de teste dos casos de falha (fatia 4a do SDD 17): o kernel lança `cases` com
//! `argv = [cases, <caso>, <número>]`; cada caso é uma função de `arch/<isa>-user` (feature `test-cases`).
//! Saídas sem caso de `asm!`: 100 sem console, 101 para `argv` inválido ou caso desconhecido.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

#[cfg(target_arch = "aarch64")]
use rustos_arch_aarch64_user::cases;
#[cfg(target_arch = "x86_64")]
use rustos_arch_x86_64_user::cases;
use rustos_user_rt::{Env, boot, entry};

entry!(main);

/// Sem console: o `hello` precisa dele para escrever.
const NO_CONSOLE: i64 = 100;
/// `argv` inválido ou caso desconhecido.
const BAD_ARGUMENTS: i64 = 101;

/// O número de um caso: decimal, ou `0` se o argumento falta.
fn decimal(text: Option<&[u8]>) -> Option<u64> {
    text.unwrap_or(&b"0"[..])
        .iter()
        .try_fold(0_u64, |acc, &byte| {
            let digit = byte.is_ascii_digit().then(|| u64::from(byte - b'0'))?;
            acc.checked_mul(10)?.checked_add(digit)
        })
}

fn main(env: &Env) -> i64 {
    let mut argv = env.args();
    let _name = argv.next();
    let Some(case) = argv.next() else {
        return BAD_ARGUMENTS;
    };
    let Some(number) = decimal(argv.next()) else {
        return BAD_ARGUMENTS;
    };
    match case {
        b"hello" => {
            let Some(console) = boot::attach_console(env).and_then(|boot| boot.console) else {
                return NO_CONSOLE;
            };
            cases::hello(console.raw())
        }
        b"fault-memory" => cases::read(number),
        b"fault-privileged" => cases::privileged(),
        b"fault-trap" => cases::trap(),
        b"fault-bad-stack" => cases::bad_stack(number),
        b"flags-ac" => cases::flags(number),
        b"null-loop" => cases::null_loop(number),
        _ => BAD_ARGUMENTS,
    }
}
