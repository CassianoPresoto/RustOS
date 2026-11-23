//! `spin`: imprime uma linha e gira para sempre, sem chamada ao kernel. É o programa que não termina
//! sozinho: o smoke o interrompe com Ctrl-C no `run` e o mata com `kill <pid>` em segundo plano.

#![no_std]
#![no_main]
#![forbid(unsafe_code)]

use rustos_user_rt::{Env, boot, entry, println};

entry!(main);

fn main(env: &Env) -> i64 {
    if boot::attach_console(env).is_none() {
        return 1;
    }
    println!("spin: spinning");
    loop {
        core::hint::spin_loop();
    }
}
