//! Os casos hostis das chamadas de handle, canal e espera (SDD 17, fatia 4c): handle forjado, velho,
//! sem direito e de outro tipo; a própria ponta no array; ponteiro ruim; tamanho acima do limite; prazo
//! vencido. Depois, com o kernel segurando a outra ponta do canal de boot, a espera que acorda por
//! sinal e não pelo prazo.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use rustos_user_rt::{Env, Handle, Rights, Status, Syscall, boot, entry, println, signals, sys};

entry!(main);

/// Endereços que o kernel tem de recusar ou não achar: sem mapa, na faixa nula, na metade do kernel.
const UNMAPPED: u64 = 0x2_0000;
const NULL_ZONE: u64 = 0x1000;
const KERNEL: u64 = 0xFFFF_8000_0000_0000;
const SECOND: u64 = 1_000_000_000;

type Raw = Result<u64, Status>;

/// Os resultados: quantos casos rodaram e o primeiro que falhou.
struct Checks {
    ran: u32,
    failed: u32,
    first: Option<&'static str>,
}

impl Checks {
    fn expect(&mut self, what: &'static str, ok: bool) {
        self.ran += 1;
        if !ok {
            self.failed += 1;
            self.first.get_or_insert(what);
            println!("handles: FAILED {what}");
        }
    }

    fn status(&mut self, what: &'static str, got: Raw, want: Status) {
        self.expect(what, got == Err(want));
    }
}

#[allow(
    unsafe_code,
    reason = "syscall crua do teste hostil; o contrato está no SAFETY"
)]
fn call(number: Syscall, args: [u64; 6]) -> Raw {
    // SAFETY: todo ponteiro que o kernel escreve é de um buffer vivo deste programa com o tamanho
    // passado, ou um endereço que o kernel recusa; nenhum aponta para memória usada de outro jeito.
    unsafe { sys::raw::syscall(number, args) }
}

fn addr(bytes: &[u8]) -> u64 {
    bytes.as_ptr().addr() as u64
}

fn words(handles: &[Handle]) -> u64 {
    handles.as_ptr().addr() as u64
}

/// `channel_write` com os argumentos crus.
fn write_raw(channel: u64, bytes: u64, len: u64, handles: u64, count: u64) -> Raw {
    call(
        Syscall::ChannelWrite,
        [channel, bytes, len, handles, count, 0],
    )
}

fn read_raw(channel: u64, bytes: u64, cap: u64, handles: u64, hcap: u64, actual: u64) -> Raw {
    call(
        Syscall::ChannelRead,
        [channel, bytes, cap, handles, hcap, actual],
    )
}

fn pair() -> (Handle, Handle) {
    sys::channel_create().unwrap_or((Handle::INVALID, Handle::INVALID))
}

fn lent(console: Handle) -> Handle {
    sys::handle_duplicate(console, Rights::WRITE.union(Rights::TRANSFER)).unwrap_or(Handle::INVALID)
}

/// O console por este handle ainda escreve.
fn writes(handle: Handle) -> bool {
    sys::debug_write(handle, b"probe").is_ok()
}

fn forged_handles(c: &mut Checks, console: Handle) {
    let stale = console.raw() + (1 << 16);
    let bad = [0, 0xFFFF, 1 << 40 | 1, 0xDEAD_BEEF_0003, u64::MAX, stale];
    let mut actual = [0_u8; 16];
    let buffer = [0_u8; 16];
    for value in bad {
        let tests = [
            call(Syscall::HandleClose, [value, 0, 0, 0, 0, 0]),
            call(Syscall::HandleDuplicate, [value, 1, 0, 0, 0, 0]),
            call(Syscall::HandleReplace, [value, 1, 0, 0, 0, 0]),
            call(Syscall::ObjectWaitOne, [value, 1, 0, 0, 0, 0]),
            call(Syscall::DebugWrite, [value, addr(&buffer), 4, 0, 0, 0]),
            write_raw(value, addr(&buffer), 4, 0, 0),
            read_raw(
                value,
                addr(&buffer),
                16,
                0,
                0,
                actual.as_mut_ptr().addr() as u64,
            ),
        ];
        for got in tests {
            c.status("a forged handle is BadHandle", got, Status::BadHandle);
        }
    }
    c.expect("the console is still there", writes(console));
}

fn old_handles(c: &mut Checks) {
    let (a, b) = pair();
    c.expect("the first close works", sys::handle_close(a).is_ok());
    c.status(
        "closing twice",
        sys::handle_close(a).map(|()| 0),
        Status::BadHandle,
    );
    let (c1, c2) = pair();
    let _ = (c1, c2);
    c.status(
        "a closed handle stays dead after the slot is reused",
        sys::object_wait_one(a, signals::READABLE, 0).map(u64::from),
        Status::BadHandle,
    );
    c.expect(
        "the peer of a closed end sees PEER_CLOSED",
        sys::object_wait_one(b, signals::PEER_CLOSED, 0)
            .is_ok_and(|seen| seen & signals::PEER_CLOSED != 0),
    );
    for handle in [b, c1, c2] {
        let _ = sys::handle_close(handle);
    }
}

fn rights(c: &mut Checks, console: Handle) {
    let can_duplicate =
        sys::handle_duplicate(console, Rights::DUPLICATE).unwrap_or(Handle::INVALID);
    c.status(
        "debug_write without WRITE",
        sys::debug_write(can_duplicate, b"x").map(|n| n as u64),
        Status::AccessDenied,
    );
    let only_transfer = sys::handle_replace(
        sys::handle_duplicate(console, Rights::TRANSFER.union(Rights::DUPLICATE))
            .unwrap_or(Handle::INVALID),
        Rights::TRANSFER,
    )
    .unwrap_or(Handle::INVALID);
    c.status(
        "handle_duplicate without DUPLICATE",
        sys::handle_duplicate(only_transfer, Rights::TRANSFER).map(Handle::raw),
        Status::AccessDenied,
    );
    let (a, b) = pair();
    let read_only = sys::handle_replace(a, Rights::READ).unwrap_or(Handle::INVALID);
    let buffer = [0_u8; 4];
    c.status(
        "channel_write without WRITE",
        write_raw(read_only.raw(), addr(&buffer), 4, 0, 0),
        Status::AccessDenied,
    );
    c.status(
        "object_wait_one without WAIT",
        sys::object_wait_one(read_only, signals::READABLE, 0).map(u64::from),
        Status::AccessDenied,
    );
    let write_only = sys::handle_replace(b, Rights::WRITE).unwrap_or(Handle::INVALID);
    let mut actual = [0_u8; 16];
    c.status(
        "channel_read without READ",
        read_raw(
            write_only.raw(),
            addr(&buffer),
            4,
            0,
            0,
            actual.as_mut_ptr().addr() as u64,
        ),
        Status::AccessDenied,
    );
    c.status(
        "debug_write on a channel",
        sys::debug_write(write_only, b"x").map(|n| n as u64),
        Status::WrongType,
    );
    c.status(
        "channel_write on the console",
        write_raw(console.raw(), addr(&buffer), 4, 0, 0),
        Status::WrongType,
    );
    let wide = lent(console);
    c.status(
        "handle_replace with rights it does not have",
        sys::handle_replace(wide, Rights::WRITE.union(Rights::READ)).map(Handle::raw),
        Status::InvalidArgs,
    );
    c.status(
        "the old handle dies even when replace fails",
        sys::handle_close(wide).map(|()| 0),
        Status::BadHandle,
    );
    for bits in [1_u64 << 11, 1 << 20, 1 << 32, u64::MAX] {
        c.status(
            "reserved right bits",
            call(Syscall::HandleDuplicate, [console.raw(), bits, 0, 0, 0, 0]),
            Status::InvalidArgs,
        );
        let spare = lent(console);
        c.status(
            "reserved right bits in replace",
            call(Syscall::HandleReplace, [spare.raw(), bits, 0, 0, 0, 0]),
            Status::InvalidArgs,
        );
        c.status(
            "and the old handle is gone anyway",
            sys::handle_close(spare).map(|()| 0),
            Status::BadHandle,
        );
    }
    c.status(
        "replace of a handle that does not exist, with bad rights",
        call(Syscall::HandleReplace, [0xBAD0_0001, 1 << 20, 0, 0, 0, 0]),
        Status::BadHandle,
    );
    c.expect("the console survives them", writes(console));
    for handle in [can_duplicate, only_transfer, read_only, write_only] {
        let _ = sys::handle_close(handle);
    }
}

fn transfers(c: &mut Checks, console: Handle) {
    let (a, b) = pair();
    let a2 = sys::handle_duplicate(a, Rights::CHANNEL).unwrap_or(Handle::INVALID);
    c.status(
        "an end carrying itself",
        sys::channel_write(a, b"x", &[a2]).map(|()| 0),
        Status::NotSupported,
    );
    c.status(
        "the array is consumed on that error",
        sys::handle_close(a2).map(|()| 0),
        Status::BadHandle,
    );
    c.expect(
        "the end that wrote is still there",
        sys::object_wait_one(a, signals::WRITABLE, 0).is_ok(),
    );
    let twice = lent(console);
    c.status(
        "the same handle twice in the array",
        sys::channel_write(a, b"x", &[twice, twice]).map(|()| 0),
        Status::BadHandle,
    );
    c.expect("and it is gone", sys::handle_close(twice).is_err());
    let plain = sys::handle_duplicate(console, Rights::WRITE.union(Rights::DUPLICATE))
        .and_then(|handle| sys::handle_replace(handle, Rights::WRITE))
        .unwrap_or(Handle::INVALID);
    c.status(
        "a handle without TRANSFER",
        sys::channel_write(a, b"x", &[plain]).map(|()| 0),
        Status::AccessDenied,
    );
    c.expect("consumed too", sys::handle_close(plain).is_err());
    c.expect(
        "nothing reached the peer",
        sys::object_wait_one(b, signals::READABLE, 0).is_err(),
    );
    let _ = (sys::handle_close(a), sys::handle_close(b));
    inside_a_dead_queue(c);
}

/// Uma ponta que viaja numa mensagem cujo receptor fecha sem ler: a fila morta se esvazia e a
/// ponta que viajava morre, e a outra ponta do canal vê `PEER_CLOSED`.
fn inside_a_dead_queue(c: &mut Checks) {
    let (x1, x2) = pair();
    let (y1, y2) = pair();
    c.expect(
        "writing an end inside a message",
        sys::channel_write(y1, b"carry", &[x2]).is_ok(),
    );
    c.expect("the end left the table", sys::handle_close(x2).is_err());
    c.expect(
        "the other end of the channel is alive while the message waits",
        sys::object_wait_one(x1, signals::PEER_CLOSED, 0).is_err(),
    );
    c.expect("closing the receiver unread", sys::handle_close(y2).is_ok());
    c.expect(
        "the end inside the message died with the queue",
        sys::object_wait_one(x1, signals::PEER_CLOSED, 0)
            .is_ok_and(|seen| seen & signals::PEER_CLOSED != 0),
    );
    c.expect(
        "and the sender of the message sees its peer gone",
        sys::object_wait_one(y1, signals::PEER_CLOSED, 0)
            .is_ok_and(|seen| seen & signals::PEER_CLOSED != 0),
    );
    let _ = (sys::handle_close(x1), sys::handle_close(y1));
}

fn bad_pointers_and_sizes(c: &mut Checks, console: Handle) {
    let (a, b) = pair();
    let message = [7_u8; 2048];
    let held = lent(console);
    for ptr in [UNMAPPED, NULL_ZONE, 0, KERNEL, u64::MAX - 7] {
        c.status(
            "an array pointer that is not the process's",
            write_raw(a.raw(), addr(&message), 1, ptr, 2),
            Status::OutOfRange,
        );
    }
    c.expect(
        "the array was not read, so nothing was consumed",
        writes(held),
    );
    c.status(
        "more than 64 handles",
        write_raw(a.raw(), addr(&message), 1, addr(&message), 65),
        Status::InvalidArgs,
    );
    c.status(
        "an absurd handle count",
        write_raw(a.raw(), addr(&message), 1, addr(&message), u64::MAX),
        Status::InvalidArgs,
    );
    c.expect("still not consumed", writes(held));
    c.status(
        "bytes above the limit",
        write_raw(a.raw(), addr(&message), 1025, words(&[held]), 1),
        Status::InvalidArgs,
    );
    c.expect("the limit error consumes the array", !writes(held));
    let nine: [Handle; 9] = core::array::from_fn(|_| lent(console));
    c.status(
        "nine handles",
        write_raw(a.raw(), addr(&message), 1, words(&nine), 9),
        Status::InvalidArgs,
    );
    c.expect(
        "all nine are consumed",
        nine.iter().all(|&handle| !writes(handle)),
    );
    c.status(
        "a byte count that cannot be",
        write_raw(a.raw(), addr(&message), u64::MAX, 0, 0),
        Status::InvalidArgs,
    );
    let held = lent(console);
    c.status(
        "a bytes pointer that is not the process's",
        write_raw(a.raw(), UNMAPPED, 8, words(&[held]), 1),
        Status::OutOfRange,
    );
    c.expect("that error consumes the array", !writes(held));
    c.expect(
        "the biggest message goes through",
        sys::channel_write(a, &message[..1024], &[]).is_ok(),
    );
    c.expect(
        "and arrives whole",
        sys::channel_read(b, &mut [0_u8; 1024], &mut []).is_ok_and(|got| got.bytes == 1024),
    );
    c.expect(
        "an empty message is valid",
        sys::channel_write(a, &[], &[]).is_ok(),
    );
    c.expect(
        "empty comes out empty",
        sys::channel_read(b, &mut [], &mut []).is_ok_and(|got| got.bytes == 0 && got.handles == 0),
    );
    let _ = (sys::handle_close(a), sys::handle_close(b));
}

fn reads(c: &mut Checks, console: Handle) {
    let (a, b) = pair();
    let mut actual = [0_u8; 16];
    let at = actual.as_mut_ptr().addr() as u64;
    let message = [9_u8; 64];
    let moved = lent(console);
    c.expect(
        "writing with a handle",
        sys::channel_write(a, &message, &[moved]).is_ok(),
    );
    let mut small = [0_u8; 8];
    let too_small = sys::channel_read(b, &mut small, &mut [Handle::INVALID; 1]);
    c.expect(
        "a buffer smaller than the message",
        too_small.is_err_and(|e| {
            e.status == Status::BufferTooSmall && e.needs.bytes == 64 && e.needs.handles == 1
        }),
    );
    let mut big = [0_u8; 64];
    let mut out = [Handle::INVALID; 2];
    c.expect(
        "too few handle slots",
        sys::channel_read(b, &mut big, &mut []).is_err_and(|e| e.status == Status::BufferTooSmall),
    );
    for (bytes, handles, actual_at) in [
        (UNMAPPED, words(&out), at),
        (addr(&big), UNMAPPED, at),
        (addr(&big), words(&out), UNMAPPED),
        (addr(&big), words(&out), 0),
        (KERNEL, words(&out), at),
    ] {
        c.status(
            "a read that cannot copy out",
            read_raw(b.raw(), bytes, 64, handles, 2, actual_at),
            Status::OutOfRange,
        );
    }
    let got = sys::channel_read(b, &mut big, &mut out);
    c.expect(
        "the message was still there, whole, after every failed copy",
        got.is_ok_and(|got| got.bytes == 64 && got.handles == 1) && big == message,
    );
    c.expect("with the handle", writes(out[0]) && out[0] != moved);
    c.status(
        "nothing left to read",
        sys::channel_read(b, &mut big, &mut out)
            .map(|_| 0)
            .map_err(|e| e.status),
        Status::ShouldWait,
    );
    let held = lent(console);
    for _ in 0..16 {
        c.expect(
            "filling the queue",
            sys::channel_write(a, b"q", &[]).is_ok(),
        );
    }
    c.status(
        "a full queue",
        sys::channel_write(a, b"q", &[held]).map(|()| 0),
        Status::ShouldWait,
    );
    c.expect("ShouldWait keeps the handle", writes(held));
    c.expect(
        "no longer writable",
        sys::object_wait_one(a, signals::WRITABLE, 0).is_err(),
    );
    c.expect(
        "one read frees one place",
        sys::channel_read(b, &mut big, &mut []).is_ok(),
    );
    c.expect(
        "writable again",
        sys::object_wait_one(a, signals::WRITABLE, 0).is_ok(),
    );
    c.expect(
        "and the write goes",
        sys::channel_write(a, b"q", &[held]).is_ok(),
    );
    c.expect("moving the handle", !writes(held));
    let _ = sys::handle_close(a);
    for _ in 0..16 {
        c.expect(
            "the writer closed, the queue is still readable",
            sys::channel_read(b, &mut big, &mut out).is_ok(),
        );
    }
    c.expect(
        "then PeerClosed",
        sys::channel_read(b, &mut big, &mut out).is_err_and(|e| e.status == Status::PeerClosed),
    );
    c.expect(
        "writing to a closed peer",
        sys::channel_write(b, b"x", &[]).is_err_and(|status| status == Status::PeerClosed),
    );
    let _ = sys::handle_close(b);
    let _ = sys::handle_close(out[0]);
}

fn creation(c: &mut Checks) {
    let mut out = [0_u8; 16];
    let at = out.as_mut_ptr().addr() as u64;
    c.status(
        "channel_create with flags",
        call(Syscall::ChannelCreate, [1, at, 0, 0, 0, 0]),
        Status::InvalidArgs,
    );
    for ptr in [UNMAPPED, NULL_ZONE, KERNEL, 0] {
        c.status(
            "channel_create with an out pointer that is not ours",
            call(Syscall::ChannelCreate, [0, ptr, 0, 0, 0, 0]),
            Status::OutOfRange,
        );
    }
    let mut held = [Handle::INVALID; 256];
    let mut made = 0;
    for pair in held.as_chunks_mut::<2>().0.iter_mut() {
        if let Ok((first, second)) = sys::channel_create() {
            pair[0] = first;
            pair[1] = second;
            made += 2;
        }
    }
    c.expect(
        "the charge stops the ends at the byte cap, never past 256, and a failed creation charges nothing",
        (100..=256).contains(&made),
    );
    c.status(
        "the end after the cap",
        sys::channel_create().map(|_| 0),
        Status::NoMemory,
    );
    for handle in held {
        let _ = sys::handle_close(handle);
    }
    c.expect(
        "closing gives the charge back",
        sys::channel_create().is_ok(),
    );
}

/// Duplica o console até a tabela recusar; os handles ficam, e a morte do processo os fecha.
fn table_limit(c: &mut Checks, console: Handle) {
    let mut made = 0_u32;
    let status = loop {
        match sys::handle_duplicate(console, Rights::WRITE) {
            Ok(_) => made += 1,
            Err(status) => break status,
        }
    };
    c.expect(
        "the table stops at 4096 handles with NoMemory",
        status == Status::NoMemory && made > 4000,
    );
}

fn waits(c: &mut Checks, console: Handle) {
    let (a, b) = pair();
    c.status(
        "an expired deadline on an empty channel",
        sys::object_wait_one(b, signals::READABLE, 0).map(u64::from),
        Status::TimedOut,
    );
    let start = sys::clock_get_monotonic().unwrap_or(0);
    let deadline = start + 5_000_000;
    c.status(
        "a deadline in the future",
        sys::object_wait_one(b, signals::READABLE, deadline).map(u64::from),
        Status::TimedOut,
    );
    c.expect(
        "it waited until the deadline",
        sys::clock_get_monotonic().unwrap_or(0) >= deadline,
    );
    c.status(
        "no signal",
        call(Syscall::ObjectWaitOne, [b.raw(), 0, 0, 0, 0, 0]),
        Status::InvalidArgs,
    );
    c.status(
        "a signal that does not exist",
        call(Syscall::ObjectWaitOne, [b.raw(), 1 << 8, 0, 0, 0, 0]),
        Status::InvalidArgs,
    );
    c.status(
        "signals beyond 32 bits",
        call(Syscall::ObjectWaitOne, [b.raw(), 1 << 32, 0, 0, 0, 0]),
        Status::InvalidArgs,
    );
    c.status(
        "the console has no WAIT",
        sys::object_wait_one(console, signals::READABLE, 0).map(u64::from),
        Status::AccessDenied,
    );
    let wanted = signals::READABLE | signals::PEER_CLOSED;
    let _ = sys::channel_write(a, b"x", &[]);
    c.expect(
        "a signal that already holds returns at once, with all of them",
        sys::object_wait_one(b, wanted, 0)
            .is_ok_and(|seen| seen == signals::READABLE | signals::WRITABLE),
    );
    let _ = sys::handle_close(a);
    c.expect(
        "and PEER_CLOSED does not hide READABLE",
        sys::object_wait_one(b, wanted, 0)
            .is_ok_and(|seen| seen & (signals::READABLE | signals::PEER_CLOSED) == wanted),
    );
    let _ = sys::handle_close(b);
}

/// Com o kernel segurando a outra ponta do canal de boot: bloqueia sem prazo útil e só volta quando
/// o kernel escreve e depois fecha a ponta dele.
fn blocking(c: &mut Checks, channel: Handle) {
    c.expect(
        "telling the kernel",
        sys::channel_write(channel, b"ready", &[]).is_ok(),
    );
    let start = sys::clock_get_monotonic().unwrap_or(0);
    let woke = sys::object_wait_one(channel, signals::READABLE, u64::MAX - 1);
    let waited = sys::clock_get_monotonic()
        .unwrap_or(0)
        .saturating_sub(start);
    c.expect(
        "a write wakes the wait, long before the deadline",
        woke.is_ok_and(|seen| seen & signals::READABLE != 0) && waited < 5 * SECOND,
    );
    c.expect("the wake did not come early", waited >= 10_000_000);
    let mut bytes = [0_u8; 8];
    c.expect(
        "the message is there",
        sys::channel_read(channel, &mut bytes, &mut []).is_ok_and(|got| got.bytes == 2)
            && &bytes[..2] == b"go",
    );
    c.expect(
        "telling the kernel again",
        sys::channel_write(channel, b"ready2", &[]).is_ok(),
    );
    let start = sys::clock_get_monotonic().unwrap_or(0);
    let closed = sys::object_wait_one(channel, signals::PEER_CLOSED, 1 << 62);
    let waited = sys::clock_get_monotonic()
        .unwrap_or(0)
        .saturating_sub(start);
    c.expect(
        "the peer closing wakes the wait",
        closed.is_ok_and(|seen| seen & signals::PEER_CLOSED != 0) && waited < 5 * SECOND,
    );
    println!(
        "handles: the peer closing woke the wait after {} us",
        waited / 1000
    );
}

/// Nanossegundos por volta de `body`, em `rounds` voltas.
fn per_round(rounds: u64, mut body: impl FnMut()) -> u64 {
    let start = sys::clock_get_monotonic().unwrap_or(0);
    for _ in 0..rounds {
        body();
    }
    sys::clock_get_monotonic()
        .unwrap_or(0)
        .saturating_sub(start)
        * 10
        / rounds
}

/// O custo no mesmo processo (o kernel ainda não tem dois): a chamada nula, `duplicate` mais `close`
/// e a ida e volta de 64 bytes por um canal (`write`, `read`, `write`, `read`). Décimos de ns.
fn bench(c: &mut Checks, console: Handle) {
    let Ok((a, b)) = sys::channel_create() else {
        return;
    };
    let message = [3_u8; 64];
    let mut back = [0_u8; 64];
    let null = per_round(2000, || {
        let _ = sys::nop();
    });
    let pair = per_round(1000, || {
        if let Ok(copy) = sys::handle_duplicate(console, Rights::WRITE) {
            let _ = sys::handle_close(copy);
        }
    });
    let round_trip = per_round(1000, || {
        let _ = sys::channel_write(a, &message, &[]);
        let _ = sys::channel_read(b, &mut back, &mut []);
        let _ = sys::channel_write(b, &message, &[]);
        let _ = sys::channel_read(a, &mut back, &mut []);
    });
    c.expect("the benchmark moved the bytes", back == message);
    println!(
        "handles: null call {}.{} ns, duplicate+close {}.{} ns, 64 B channel round trip {}.{} ns (same process, 4 calls)",
        null / 10,
        null % 10,
        pair / 10,
        pair % 10,
        round_trip / 10,
        round_trip % 10
    );
    let _ = (sys::handle_close(a), sys::handle_close(b));
}

fn main(env: &Env) -> i64 {
    let Some(boot) = boot::attach_console(env) else {
        return 100;
    };
    let (Some(console), Some(channel)) = (boot.console, env.boot_channel()) else {
        return 100;
    };
    let mut c = Checks {
        ran: 0,
        failed: 0,
        first: None,
    };
    forged_handles(&mut c, console);
    old_handles(&mut c);
    rights(&mut c, console);
    transfers(&mut c, console);
    bad_pointers_and_sizes(&mut c, console);
    reads(&mut c, console);
    creation(&mut c);
    waits(&mut c, console);
    bench(&mut c, console);
    blocking(&mut c, channel);
    table_limit(&mut c, console);
    if c.failed == 0 {
        println!("handles: ok ({} assertions)", c.ran);
        0
    } else {
        println!("handles: FAILED {} of {} assertions", c.failed, c.ran);
        i64::from(c.failed)
    }
}
