//! `count`: conta de 1 a 5 com meio segundo entre os números, dormindo em `object_wait_one` com
//! prazo sobre o próprio processo (que nunca termina por dentro). Com `run count &`, o `ps` o mostra
//! esperando enquanto isso.

#![no_std]
#![no_main]
#![forbid(unsafe_code)]

use rustos_user_rt::{Env, Status, boot, entry, println, signals, sys};

entry!(main);

const TICKS: u64 = 5;
const STEP_NS: u64 = 500_000_000;

fn main(env: &Env) -> i64 {
    let Some(process) = boot::attach_console(env).and_then(|boot| boot.process) else {
        return 1;
    };
    for tick in 1..=TICKS {
        let Ok(now) = sys::clock_get_monotonic() else {
            return 2;
        };
        match sys::object_wait_one(process, signals::TERMINATED, now + STEP_NS) {
            Err(Status::TimedOut) => {}
            _ => return 3,
        }
        println!(
            "count: {tick}/{TICKS} (up {} ms)",
            (now + STEP_NS) / 1_000_000
        );
    }
    println!("count: done");
    0
}
