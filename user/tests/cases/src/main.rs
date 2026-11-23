//! Programa de teste dos casos de falha de usuário (fatia 4a do SDD 17): o kernel lança `cases` com
//! `argv = [cases, <caso>, <arg0>]` e confere o evento que o processo deixa. As instruções de cada
//! caso moram em `arch/<isa>-user` (feature `test-cases`); aqui só se escolhe o caso.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

#[cfg(target_arch = "aarch64")]
use rustos_arch_aarch64_user::cases;
#[cfg(target_arch = "x86_64")]
use rustos_arch_x86_64_user::cases;
use rustos_user_rt::{Env, boot, entry};

entry!(main);

/// O `arg0` de um caso: um número decimal, ou `0` se o argumento falta.
fn decimal(text: Option<&[u8]>) -> Option<u64> {
    text.unwrap_or(&b"0"[..])
        .iter()
        .try_fold(0_u64, |acc, &byte| {
            let digit = byte.is_ascii_digit().then(|| u64::from(byte - b'0'))?;
            acc.checked_mul(10)?.checked_add(digit)
        })
}

fn main(env: &Env) -> i64 {
    let mut args = env.args();
    let _name = args.next();
    let Some(case) = args.next() else {
        return 2;
    };
    let Some(arg0) = decimal(args.next()) else {
        return 2;
    };
    match case {
        b"hello" => {
            let Some(console) = boot::attach_console(env).and_then(|boot| boot.console) else {
                return 1;
            };
            cases::hello(console.raw())
        }
        b"fault-memory" => cases::read(arg0),
        b"fault-privileged" => cases::privileged(),
        b"fault-trap" => cases::trap(),
        b"fault-bad-stack" => cases::bad_stack(),
        b"flags-ac" => cases::flags(arg0),
        b"null-loop" => cases::null_loop(arg0),
        _ => 2,
    }
}
