//! O programa filho (SDD 17, fatia 4c, parte 3): quem o cria escreve a mensagem de boot (o console
//! que ele usa, o processo e o VMAR raiz dele) e, se quiser, uma segunda mensagem com o modo. Sem
//! segunda mensagem ele diz olá pelo console e sai. Uma thread que nasce com `arg0` 0 gira para
//! sempre, e com `arg0` 7 sai logo (`thread_exit`): são as threads a mais dos testes de morte.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::hint::black_box;

use rustos_user_rt::boot::{self, Boot};
use rustos_user_rt::vm;
use rustos_user_rt::{Env, Handle, entry, entry_address, println, signals, sys};

entry!(main);

/// Modos da segunda mensagem: o primeiro byte.
const MODE_QUIET: u8 = 1;
const MODE_SPIN: u8 = 2;
const MODE_BLOCK: u8 = 3;
const MODE_OVERFLOW: u8 = 4;
const MODE_ECHO: u8 = 5;
const MODE_EXIT: u8 = 6;

/// O `arg0` das threads a mais: gira para sempre, ou sai.
const WORKER_SPIN: u64 = 0;
const WORKER_EXIT: u64 = 7;

/// Recursão sem fim: a pilha de 64 KiB acaba na página sem mapa e o processo morre de falta de
/// página, o estouro de pilha contido.
#[inline(never)]
#[allow(
    unconditional_recursion,
    reason = "a recursão sem fim é o caso: o estouro de pilha cai na página sem mapa"
)]
fn recurse(depth: u64) -> u64 {
    let mut pad = [0_u8; 512];
    pad[(depth % 512) as usize] = depth as u8;
    black_box(&mut pad);
    recurse(black_box(depth + 1)) + u64::from(pad[0])
}

fn spin() -> ! {
    loop {
        black_box(0_u8);
        core::hint::spin_loop();
    }
}

/// A segunda mensagem do pai, sem bloquear: o modo e o byte seguinte.
fn mode(channel: Handle) -> Option<(u8, u8)> {
    let mut bytes = [0_u8; 8];
    let actual = sys::channel_read(channel, &mut bytes, &mut []).ok()?;
    (actual.bytes >= 1).then(|| (bytes[0], bytes.get(1).copied().unwrap_or(0)))
}

/// Eco: devolve cada mensagem que chega ao pai, `rounds` vezes.
fn echo(channel: Handle, rounds: u32) -> i64 {
    let mut buffer = [0_u8; 8];
    for _ in 0..rounds {
        if sys::object_wait_one(channel, signals::READABLE, u64::MAX).is_err() {
            return 90;
        }
        let Ok(actual) = sys::channel_read(channel, &mut buffer, &mut []) else {
            return 91;
        };
        let len = usize::try_from(actual.bytes).unwrap_or(0);
        if sys::channel_write(channel, &buffer[..len], &[]).is_err() {
            return 92;
        }
    }
    0
}

/// O que o filho confere do que recebeu e o que ele mesmo prova: o console, o auxv, a pilha e o VMAR
/// raiz dele (mapeia um VMO, escreve e lê).
fn hello(env: &Env, boot: Boot) -> i64 {
    let mut code = 0;
    let own_vmar = boot.vmar_root.ok_or(()).and_then(|vmar| {
        let (_vmo, mut map) = vm::scratch(vmar, 8192).map_err(|_| ())?;
        map.bytes_mut()[4097] = 0x5A;
        (map.bytes()[4097] == 0x5A).then_some(()).ok_or(())
    });
    for (number, (what, ok)) in (1_i64..).zip([
        ("argc is 0", env.argc() == 0),
        ("sp is 16-byte aligned", env.initial_sp().is_multiple_of(16)),
        ("boot handle is 1", env.boot_handle() == 1),
        ("AT_ENTRY is _start", env.auxv().entry == entry_address()),
        ("a process handle in the boot message", boot.process.is_some()),
        ("the root VMAR mapping a VMO of its own", own_vmar.is_ok()),
    ]) {
        if !ok {
            println!("child: FAILED {what}");
            code = code.max(number);
        }
    }
    if code == 0 {
        println!("child: hello from a child process, the console came by the boot channel");
    }
    code
}

fn main(env: &Env) -> i64 {
    match env.boot_handle() {
        WORKER_SPIN => spin(),
        WORKER_EXIT => sys::thread_exit(),
        _ => {}
    }
    let Some(channel) = env.boot_channel() else {
        return 98;
    };
    let Ok(boot) = boot::read(channel) else {
        return 97;
    };
    if let Some(console) = boot.console {
        rustos_user_rt::set_console(console);
    }
    match mode(channel) {
        None => hello(env, boot),
        Some((MODE_QUIET, _)) => 0,
        Some((MODE_SPIN, _)) => spin(),
        Some((MODE_BLOCK, _)) => {
            let _ = sys::object_wait_one(channel, signals::PEER_CLOSED, u64::MAX);
            0
        }
        Some((MODE_OVERFLOW, _)) => i64::try_from(recurse(0)).unwrap_or(0),
        Some((MODE_ECHO, rounds)) => echo(channel, u32::from(rounds) * 16),
        Some((MODE_EXIT, code)) => i64::from(code),
        Some(_) => 96,
    }
}
