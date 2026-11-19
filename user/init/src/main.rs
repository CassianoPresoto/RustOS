//! O `init`: o primeiro programa de usuário. Confere o que o kernel deixou na pilha inicial
//! (auxv, alinhamento, handle de boot), lê a mensagem de boot e escreve pelo console que ela traz,
//! exercita canal e espera com um canal próprio, cria um processo filho a partir do initramfs (lê o ELF
//! pelo VMO, mapeia os segmentos, escreve a mensagem de boot com o console e o inicia), espera o
//! `TERMINATED` dele, chama `clock_get_monotonic` e `random_get`, e sai com 0.

#![no_std]
#![no_main]
#![forbid(unsafe_code)]

use rustos_user_rt::boot::{self, Boot};
use rustos_user_rt::spawn::{self, Parent};
use rustos_user_rt::vm::Archive;
use rustos_user_rt::{
    Env, Handle, Rights, Status, boot_kind, entry, entry_address, println, signals, sys,
};

entry!(main);

/// `PT_LOAD` de um executável com código e dados em páginas separadas: pelo menos dois.
const MIN_LOADS: usize = 2;

/// O que não bateu, na ordem em que o `init` confere; o código de saída é o primeiro.
fn checks(env: &Env) -> [(&'static str, bool); 8] {
    let aux = env.auxv();
    [
        ("argc is 0", env.argc() == 0),
        ("sp is 16-byte aligned", env.initial_sp().is_multiple_of(16)),
        ("boot handle is the first handle, 1", env.boot_handle() == 1),
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

/// Espera até um segundo por `signal` em `handle`.
fn wait_for(handle: Handle, signal: u32) -> bool {
    let Ok(now) = sys::clock_get_monotonic() else {
        return false;
    };
    sys::object_wait_one(handle, signal, now + 1_000_000_000).is_ok_and(|seen| seen & signal != 0)
}

/// Lê uma mensagem de `channel` e devolve os bytes e o primeiro handle que ela traz.
fn receive(channel: Handle) -> Option<([u8; 64], usize, Option<Handle>)> {
    let mut bytes = [0_u8; 64];
    let mut handles = [Handle::INVALID; 2];
    let actual = sys::channel_read(channel, &mut bytes, &mut handles).ok()?;
    let len = usize::try_from(actual.bytes).ok()?;
    let first = (actual.handles > 0).then_some(handles[0]);
    Some((bytes, len, first))
}

/// A mensagem de boot e o console por handle: o que o kernel entregou, com os direitos que disse.
fn boot_channel(env: &Env, boot: Boot) -> Result<Handle, &'static str> {
    let channel = env.boot_channel().ok_or("a boot channel")?;
    let (process, console) = boot
        .process
        .zip(boot.console)
        .ok_or("process and console")?;
    let closed = sys::object_wait_one(channel, signals::PEER_CLOSED, 0);
    if !closed.is_ok_and(|seen| seen & signals::PEER_CLOSED != 0) {
        return Err("the kernel closing its end of the boot channel");
    }
    let again = sys::channel_read(channel, &mut [], &mut []);
    if again.err().map(|error| error.status) != Some(Status::PeerClosed) {
        return Err("the boot message read only once");
    }
    let wide = Rights::WRITE.union(Rights::READ);
    if sys::handle_duplicate(console, wide) != Err(Status::InvalidArgs) {
        return Err("the console refusing a right it does not have");
    }
    if sys::object_wait_one(process, signals::TERMINATED, 0) != Err(Status::TimedOut) {
        return Err("the init not being terminated");
    }
    Ok(console)
}

/// Canal, espera e transferência com um canal próprio: escreve, espera `READABLE`, lê, passa um
/// handle de ida e o recebe de volta.
fn own_channel(console: Handle) -> Result<(), &'static str> {
    let (a, b) = sys::channel_create().map_err(|_| "channel_create")?;
    if sys::object_wait_one(b, signals::READABLE, 0) != Err(Status::TimedOut) {
        return Err("an empty channel not being readable");
    }
    let message = [0x5A_u8; 64];
    sys::channel_write(a, &message, &[]).map_err(|_| "channel_write")?;
    if !wait_for(b, signals::READABLE) {
        return Err("READABLE after a write");
    }
    let (got, len, none) = receive(b).ok_or("channel_read")?;
    if len != 64 || got != message || none.is_some() {
        return Err("the 64 bytes coming back whole");
    }
    let lent = sys::handle_duplicate(console, Rights::WRITE.union(Rights::TRANSFER))
        .map_err(|_| "handle_duplicate")?;
    sys::channel_write(a, b"take", &[lent]).map_err(|_| "channel_write with a handle")?;
    if sys::handle_close(lent) != Err(Status::BadHandle) {
        return Err("a transferred handle leaving the table");
    }
    if !wait_for(b, signals::READABLE) {
        return Err("READABLE after the transfer");
    }
    let (_, _, moved) = receive(b).ok_or("channel_read with a handle")?;
    let moved = moved
        .filter(|&moved| moved != lent)
        .ok_or("a new handle value")?;
    sys::debug_write(moved, b"init: a transferred console handle works")
        .map_err(|_| "the console through the moved handle")?;
    sys::channel_write(b, b"back", &[moved]).map_err(|_| "sending the handle back")?;
    if !wait_for(a, signals::READABLE) {
        return Err("READABLE on the first end");
    }
    let (_, _, back) = receive(a).ok_or("channel_read of the handle coming back")?;
    let back = back.ok_or("the handle coming back")?;
    sys::debug_write(back, b"init: the handle came back").map_err(|_| "the handle back")?;
    sys::handle_close(back).map_err(|_| "handle_close")?;
    sys::handle_close(a).map_err(|_| "handle_close of the first end")?;
    if !wait_for(b, signals::PEER_CLOSED) {
        return Err("PEER_CLOSED after closing the other end");
    }
    sys::handle_close(b).map_err(|_| "handle_close of the second end")
}

/// O filho: lê o `child` do initramfs (o VMO que o kernel entregou), cria o processo, mapeia os
/// segmentos, monta a pilha, escreve a mensagem de boot com o console e o inicia; depois espera o
/// `TERMINATED` dele. O filho escreve pelo console que recebeu e sai.
fn child(boot: Boot) -> Result<(), &'static str> {
    let parent = Parent::from_boot(&boot).ok_or("a process and a root VMAR in the boot message")?;
    let console = boot.console.ok_or("a console in the boot message")?;
    let Some(initramfs) = boot.initramfs else {
        println!("init: no initramfs in the boot message, no child");
        return Ok(());
    };
    let archive = Archive::open(parent.vmar, initramfs).map_err(|_| "mapping the initramfs")?;
    let file = archive.file(b"child").ok_or("the child in the initramfs")?;
    let child = spawn::create(parent, file).map_err(|_| "creating the child process")?;
    let lent = sys::handle_duplicate(console, Rights::WRITE.union(Rights::TRANSFER))
        .map_err(|_| "duplicating the console for the child")?;
    child
        .send_boot(&[(boot_kind::CONSOLE, lent)])
        .map_err(|_| "writing the boot message")?;
    let sp = child.stack().map_err(|_| "building the child stack")?;
    let thread = child.start(sp).map_err(|_| "starting the child")?;
    let now = sys::clock_get_monotonic().map_err(|_| "the clock")?;
    let waited = child.wait(now + 5_000_000_000);
    let _ = sys::handle_close(thread);
    child.release();
    waited.map_err(|_| "the child terminating")?;
    println!("init: the child process ran and terminated");
    Ok(())
}

fn handles(env: &Env, boot: Boot) -> bool {
    let result = boot_channel(env, boot).and_then(own_channel);
    match result {
        Ok(()) => {
            println!("init: boot channel, console and handles ok");
            true
        }
        Err(what) => {
            println!("init: FAILED {what}");
            false
        }
    }
}

/// Código de saída quando o `init` não consegue anexar o console: sem ele nenhuma linha sai, então
/// o número que o kernel registra (`pid 1 exited (code 99`) é o diagnóstico.
const EXIT_NO_CONSOLE: i64 = 99;

fn main(env: &Env) -> i64 {
    let Some(boot) = boot::attach_console(env) else {
        return EXIT_NO_CONSOLE;
    };
    let mut code = 0;
    for (number, (what, ok)) in (1_i64..).zip(checks(env)) {
        if !ok {
            println!("init: FAILED {what}");
            if code == 0 {
                code = number;
            }
        }
    }
    if !handles(env, boot) && code == 0 {
        code = 101;
    }
    if let Err(what) = child(boot) {
        println!("init: FAILED {what}");
        if code == 0 {
            code = 102;
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
