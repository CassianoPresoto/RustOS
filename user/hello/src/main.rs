//! `hello`: o programa mais curto que o shell lança. Diz olá pelo console de depuração e sai com 0.

#![no_std]
#![no_main]
#![forbid(unsafe_code)]

use rustos_user_rt::{Env, boot, entry, println};

entry!(main);

const ARCH: &str = if cfg!(target_arch = "x86_64") {
    "ring 3 on x86_64"
} else {
    "EL0 on aarch64"
};

fn main(env: &Env) -> i64 {
    if boot::attach_console(env).is_none() {
        return 1;
    }
    println!("hello from a user process ({ARCH})");
    0
}
