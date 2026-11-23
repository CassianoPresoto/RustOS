//! `count [n]`: conta de 1 a `n` (5 sem argumento, de 1 a 60) com meio segundo entre os números,
//! dormindo em `object_wait_one` com prazo sobre o próprio processo (que nunca termina por dentro).
//! Com `run count &`, o `ps` o mostra esperando enquanto isso.

#![no_std]
#![no_main]
#![forbid(unsafe_code)]

use rustos_user_rt::{Env, Status, boot, entry, println, signals, sys};

entry!(main);

/// Sem argumento, e o intervalo que aceita.
const DEFAULT_TICKS: u64 = 5;
const MAX_TICKS: u64 = 60;
const STEP_NS: u64 = 500_000_000;
const USAGE: &str = "count: usage: count [n] (1..60)";

fn main(env: &Env) -> i64 {
    let Some(process) = boot::attach_console(env).and_then(|boot| boot.process) else {
        return 1;
    };
    let Some(ticks) = ticks_of(env) else {
        println!("{USAGE}");
        return 2;
    };
    for tick in 1..=ticks {
        let Ok(now) = sys::clock_get_monotonic() else {
            return 2;
        };
        match sys::object_wait_one(process, signals::TERMINATED, now + STEP_NS) {
            Err(Status::TimedOut) => {}
            _ => return 3,
        }
        println!(
            "count: {tick}/{ticks} (up {} ms)",
            (now + STEP_NS) / 1_000_000
        );
    }
    println!("count: done");
    0
}

/// O `n` do argumento, ou o padrão sem argumento. `None` com mais de um argumento, com algo que não é
/// decimal ou fora de 1 a 60.
fn ticks_of(env: &Env) -> Option<u64> {
    let mut args = env.args();
    let _program = args.next();
    let Some(text) = args.next() else {
        return Some(DEFAULT_TICKS);
    };
    if args.next().is_some() {
        return None;
    }
    let ticks = parse_decimal(text)?;
    (1..=MAX_TICKS).contains(&ticks).then_some(ticks)
}

/// Um decimal sem sinal, só dígitos; `None` para vazio, letra ou número que não cabe em `u64`.
fn parse_decimal(text: &[u8]) -> Option<u64> {
    if text.is_empty() {
        return None;
    }
    text.iter().try_fold(0_u64, |value, &byte| {
        let digit = match byte {
            b'0'..=b'9' => u64::from(byte - b'0'),
            _ => return None,
        };
        value.checked_mul(10)?.checked_add(digit)
    })
}
