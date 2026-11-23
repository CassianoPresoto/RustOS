//! Os casos hostis de VMO, VMAR, processo e thread (SDD 17, fatia 4c, parte 3), as medidas e o laço de
//! criar e matar processos. O kernel segura a outra ponta do canal de boot: nos três pontos de
//! sincronia (`A`, `B`, `C`) ele mede os frames, o heap e a promessa, e o programa só segue com o `go`.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use rustos_user_rt::boot::{self, Boot};
use rustos_user_rt::spawn::{self, Child, Parent};
use rustos_user_rt::vm::{Archive, Mapping};
use rustos_user_rt::{
    Env, Handle, PROCESS_KILLED_CODE, ProcessEvent, Rights, Status, Syscall, boot_kind, entry,
    event_kind, fault_access, fault_kind, println, signals, sys, vmar_flags,
};

entry!(main);

const PAGE: u64 = 4096;
const MS: u64 = 1_000_000;
const SECOND: u64 = 1_000_000_000;
const RW: u32 = vmar_flags::READ | vmar_flags::WRITE;
const VMO_MAX: u64 = 256 << 20;
const USER_MIN: u64 = 0x1_0000;
const USER_TOP: u64 = 0x0000_7FFF_FFFF_F000;
const KERNEL: u64 = 0xFFFF_8000_0000_0000;
const UNMAPPED: u64 = 0x2_0000;
/// Modos que o `child` lê da segunda mensagem.
const MODE_QUIET: u8 = 1;
const MODE_SPIN: u8 = 2;
const MODE_BLOCK: u8 = 3;
const MODE_OVERFLOW: u8 = 4;
const MODE_ECHO: u8 = 5;
const MODE_SELF_KILL: u8 = 7;
/// O modo de sair com o código do segundo byte (`process_exit` do `child`).
const MODE_EXIT_CODE: u8 = 6;
const WORKER_SPIN: u64 = 0;
const WORKER_EXIT: u64 = 7;
/// Rodadas do laço de criar e matar, e idas e voltas do eco entre processos.
const LOOPS: u32 = 300;
const ECHOES: u32 = 2000;

type Raw = Result<u64, Status>;

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
            println!("procs: FAILED {what}");
        }
    }

    fn status<T>(&mut self, what: &'static str, got: Result<T, Status>, want: Status) {
        self.expect(what, got.err() == Some(want));
    }
}

/// Uma chamada com valores crus, para forjar o que os envoltórios seguros não deixam passar.
#[allow(
    unsafe_code,
    reason = "syscall crua de teste hostil (guidelines 03, regra 11)"
)]
fn raw(call: Syscall, args: [u64; 6]) -> Raw {
    // SAFETY: todo ponteiro em `args` é de um buffer vivo do chamador com o tamanho que o kernel lê
    // ou escreve, ou um endereço que o kernel recusa.
    unsafe { sys::raw::syscall(call, args) }
}

fn now() -> u64 {
    sys::clock_get_monotonic().unwrap_or(0)
}

/// O que as seções precisam do ambiente.
struct Cx<'a> {
    parent: Parent,
    console: Handle,
    initramfs: Handle,
    child: &'a [u8],
    boot_channel: Handle,
}

/// Espera `signal` em `handle` por até `ns`.
fn wait(handle: Handle, signal: u32, ns: u64) -> bool {
    sys::object_wait_one(handle, signal, now() + ns).is_ok_and(|seen| seen & signal != 0)
}

/// Dorme `ns` esperando um sinal que não vem: um canal vazio.
fn sleep(ns: u64) {
    if let Ok((a, b)) = sys::channel_create() {
        let _ = sys::object_wait_one(a, signals::READABLE, now() + ns);
        let _ = sys::handle_close(a);
        let _ = sys::handle_close(b);
    }
}

/// O ponto de sincronia com o kernel: diz `tag` e espera o `go`. Sem o kernel do outro lado o prazo
/// vence e o programa segue.
fn sync(cx: &Cx<'_>, tag: &[u8]) {
    let _ = sys::channel_write(cx.boot_channel, tag, &[]);
    if wait(cx.boot_channel, signals::READABLE, 20 * SECOND) {
        let mut reply = [0_u8; 8];
        let _ = sys::channel_read(cx.boot_channel, &mut reply, &mut []);
    }
}

/// Um filho montado com o modo e, se pedido, o console: ainda sem thread.
fn start(cx: &Cx<'_>, mode: Option<u8>, with_console: bool) -> Result<Child, spawn::Error> {
    let child = spawn::create(cx.parent, cx.child)?;
    let lent = if with_console {
        Some(
            sys::handle_duplicate(cx.console, Rights::WRITE.union(Rights::TRANSFER))
                .map_err(spawn::Error::Sys)?,
        )
    } else {
        None
    };
    let mut items = [(0_u32, Handle::INVALID)];
    let extra: &[(u32, Handle)] = if let Some(console) = lent {
        items[0] = (boot_kind::CONSOLE, console);
        &items
    } else {
        &[]
    };
    child.send_boot(extra)?;
    if let Some(mode) = mode {
        sys::channel_write(child.channel, &[mode, 0], &[]).map_err(spawn::Error::Sys)?;
    }
    Ok(child)
}

/// Os argumentos dos VMOs e VMARs que o kernel tem de recusar, e os direitos como teto.
fn vmo_vmar(c: &mut Checks, cx: &Cx<'_>) {
    let vmar = cx.parent.vmar;
    c.status(
        "vmo_create of 0 bytes",
        sys::vmo_create(0),
        Status::InvalidArgs,
    );
    c.status(
        "vmo_create unaligned",
        sys::vmo_create(100),
        Status::InvalidArgs,
    );
    c.status(
        "vmo_create above the maximum",
        sys::vmo_create(VMO_MAX + PAGE),
        Status::InvalidArgs,
    );
    c.status(
        "vmo_create with flags",
        raw(Syscall::VmoCreate, [PAGE, 1, 0, 0, 0, 0]),
        Status::InvalidArgs,
    );
    let Ok(vmo) = sys::vmo_create(4 * PAGE) else {
        c.expect("vmo_create of 4 pages", false);
        return;
    };
    let read_map = sys::handle_duplicate(vmo, Rights::READ.union(Rights::MAP));
    let write_only = sys::handle_duplicate(vmo, Rights::READ.union(Rights::WRITE));
    let exec_map = sys::handle_duplicate(vmo, Rights::EXECUTE.union(Rights::MAP));
    let read_vmar = sys::handle_duplicate(vmar, Rights::READ.union(Rights::MAP));
    let (Ok(read_map), Ok(write_only), Ok(exec_map), Ok(read_vmar)) =
        (read_map, write_only, exec_map, read_vmar)
    else {
        c.expect("handle_duplicate of the VMO and the VMAR", false);
        return;
    };
    let map = |vmar, vmo, flags| sys::vmar_map(vmar, vmo, 0, PAGE, flags, 0);
    c.status(
        "a VMO handle without WRITE mapped for writing",
        map(vmar, read_map, RW),
        Status::AccessDenied,
    );
    c.status(
        "a VMO handle without MAP",
        map(vmar, write_only, vmar_flags::READ),
        Status::AccessDenied,
    );
    c.status(
        "a VMO handle with EXECUTE but no READ, mapped for reading",
        map(vmar, exec_map, vmar_flags::READ),
        Status::AccessDenied,
    );
    c.status(
        "a VMAR handle without WRITE, mapping for writing",
        map(read_vmar, vmo, RW),
        Status::AccessDenied,
    );
    c.status(
        "the initramfs VMO mapped for writing",
        map(vmar, cx.initramfs, RW),
        Status::AccessDenied,
    );
    c.status(
        "the initramfs VMO mapped for execution",
        map(vmar, cx.initramfs, vmar_flags::READ | vmar_flags::EXECUTE),
        Status::AccessDenied,
    );
    c.status(
        "write and execute together",
        map(vmar, vmo, vmar_flags::WRITE | vmar_flags::EXECUTE),
        Status::InvalidArgs,
    );
    c.status(
        "a reserved flag bit",
        map(vmar, vmo, 1 << 9),
        Status::InvalidArgs,
    );
    wrong_handles(c, cx, vmo);
    placement(c, vmar, vmo);
}

/// Handle de outro tipo, forjado: o tipo e a existência valem antes de qualquer outro argumento.
fn wrong_handles(c: &mut Checks, cx: &Cx<'_>, vmo: Handle) {
    let vmar = cx.parent.vmar;
    let map = |vmar, vmo, flags| sys::vmar_map(vmar, vmo, 0, PAGE, flags, 0);
    c.status(
        "a VMO as the VMAR",
        map(vmo, vmo, vmar_flags::READ),
        Status::WrongType,
    );
    c.status(
        "a VMAR as the VMO",
        map(vmar, vmar, vmar_flags::READ),
        Status::WrongType,
    );
    c.status(
        "a console as the VMO",
        map(vmar, cx.console, vmar_flags::READ),
        Status::WrongType,
    );
    let forged = Handle::from_raw(0xDEAD_0001).unwrap_or(vmo);
    c.status(
        "a forged VMO",
        map(vmar, forged, vmar_flags::READ),
        Status::BadHandle,
    );
    c.status(
        "a forged VMAR",
        map(forged, vmo, vmar_flags::READ),
        Status::BadHandle,
    );
}

/// Onde o kernel põe um mapeamento e o que recusa: `FIXED` sobre região, fora da faixa, e tamanhos.
fn placement(c: &mut Checks, vmar: Handle, vmo: Handle) {
    let Ok(at) = sys::vmar_map(vmar, vmo, 0, 4 * PAGE, RW, 0) else {
        c.expect("vmar_map of 4 pages", false);
        return;
    };
    c.expect(
        "an ANY mapping above the 64 GiB floor and on the 4 KiB grid",
        at >= 0x10_0000_0000 && at.is_multiple_of(PAGE),
    );
    let fixed =
        |addr: u64, len: u64| sys::vmar_map(vmar, vmo, 0, len, RW | vmar_flags::FIXED, addr);
    c.status("FIXED over a mapping", fixed(at, PAGE), Status::InvalidArgs);
    c.status(
        "FIXED inside a mapping",
        fixed(at + PAGE, PAGE),
        Status::InvalidArgs,
    );
    c.status(
        "FIXED across the end",
        fixed(at + 3 * PAGE, 2 * PAGE),
        Status::InvalidArgs,
    );
    c.status(
        "FIXED across the start",
        fixed(at - PAGE, 2 * PAGE),
        Status::InvalidArgs,
    );
    c.status("FIXED in the null zone", fixed(0, PAGE), Status::OutOfRange);
    c.status(
        "FIXED below the minimum",
        fixed(USER_MIN - PAGE, PAGE),
        Status::OutOfRange,
    );
    c.status(
        "FIXED at the top",
        fixed(USER_TOP, PAGE),
        Status::OutOfRange,
    );
    c.status(
        "FIXED in the kernel half",
        fixed(KERNEL, PAGE),
        Status::OutOfRange,
    );
    c.status(
        "FIXED unaligned",
        fixed(at + 5 * PAGE + 8, PAGE),
        Status::InvalidArgs,
    );
    c.status(
        "an address without FIXED",
        sys::vmar_map(vmar, vmo, 0, PAGE, RW, at + 8 * PAGE),
        Status::InvalidArgs,
    );
    c.status(
        "a zero length",
        sys::vmar_map(vmar, vmo, 0, 0, RW, 0),
        Status::InvalidArgs,
    );
    c.status(
        "an unaligned length",
        sys::vmar_map(vmar, vmo, 0, 100, RW, 0),
        Status::InvalidArgs,
    );
    c.status(
        "an unaligned VMO offset",
        sys::vmar_map(vmar, vmo, 8, PAGE, RW, 0),
        Status::InvalidArgs,
    );
    c.status(
        "past the end of the VMO",
        sys::vmar_map(vmar, vmo, 3 * PAGE, 2 * PAGE, RW, 0),
        Status::OutOfRange,
    );
    c.status(
        "an offset plus a length that wraps",
        sys::vmar_map(vmar, vmo, 1 << 63, 1 << 63, RW, 0),
        Status::OutOfRange,
    );
    c.status(
        "a huge length that is not a page multiple",
        sys::vmar_map(vmar, vmo, 0, u64::MAX, RW, 0),
        Status::InvalidArgs,
    );
    unmap_rules(c, vmar, vmo, at);
}

/// `vmar_unmap`: argumentos ruins, handle sem `MAP`, buraco e o endereço que volta a ser livre.
fn unmap_rules(c: &mut Checks, vmar: Handle, vmo: Handle, at: u64) {
    let fixed =
        |addr: u64, len: u64| sys::vmar_map(vmar, vmo, 0, len, RW | vmar_flags::FIXED, addr);
    let another = sys::vmar_map(vmar, vmo, 0, PAGE, RW, 0);
    c.expect(
        "the failed mappings left the first one alone",
        another.is_ok(),
    );
    c.status(
        "vmar_unmap unaligned",
        sys::vmar_unmap(vmar, at + 8, PAGE),
        Status::InvalidArgs,
    );
    c.status(
        "vmar_unmap of zero bytes",
        sys::vmar_unmap(vmar, at, 0),
        Status::InvalidArgs,
    );
    c.status(
        "vmar_unmap above the top",
        sys::vmar_unmap(vmar, USER_TOP, PAGE),
        Status::OutOfRange,
    );
    let read_only = sys::handle_duplicate(vmar, Rights::READ).unwrap_or(vmar);
    c.status(
        "vmar_unmap with a handle that has no MAP",
        sys::vmar_unmap(read_only, at, PAGE),
        Status::AccessDenied,
    );
    c.expect(
        "vmar_unmap of a hole",
        sys::vmar_unmap(vmar, at + 64 * PAGE, PAGE).is_ok(),
    );
    c.expect(
        "vmar_unmap of the mapping",
        sys::vmar_unmap(vmar, at, 4 * PAGE).is_ok(),
    );
    if let Ok(other) = another {
        let _ = sys::vmar_unmap(vmar, other, PAGE);
    }
    c.expect(
        "the address is free again",
        fixed(at, 4 * PAGE).is_ok() && sys::vmar_unmap(vmar, at, 4 * PAGE).is_ok(),
    );
}

/// Memória compartilhada pelo VMO e o desmapeamento parcial.
fn sharing_and_partial_unmap(c: &mut Checks, cx: &Cx<'_>) {
    let vmar = cx.parent.vmar;
    let Ok(vmo) = sys::vmo_create(8 * PAGE) else {
        c.expect("vmo_create of 8 pages", false);
        return;
    };
    let len = 8 * 4096_usize;
    let (Ok(mut first), Ok(second)) = (
        Mapping::new(vmar, vmo, 0, len, RW),
        Mapping::new(vmar, vmo, 0, len, RW),
    ) else {
        c.expect("two mappings of one VMO", false);
        return;
    };
    c.expect(
        "two mappings at two addresses",
        first.addr() != second.addr(),
    );
    for page in 0..8 {
        first.bytes_mut()[page * 4096 + 7] = 0x10 + u8::try_from(page).unwrap_or(0);
    }
    c.expect(
        "a write through one mapping is seen through the other",
        (0..8)
            .all(|page| second.bytes()[page * 4096 + 7] == 0x10 + u8::try_from(page).unwrap_or(0)),
    );
    c.expect(
        "a page nobody wrote reads zero",
        second.bytes()[100] == 0 && second.bytes()[4096 + 100] == 0,
    );
    let base = first.addr();
    c.expect(
        "vmar_unmap of the middle pages",
        sys::vmar_unmap(vmar, base + 2 * PAGE, 2 * PAGE).is_ok(),
    );
    c.expect(
        "the pages around the hole still hold their values",
        first.bytes()[7] == 0x10
            && first.bytes()[4096 + 7] == 0x11
            && first.bytes()[4 * 4096 + 7] == 0x14
            && first.bytes()[7 * 4096 + 7] == 0x17,
    );
    let fixed = |addr: u64, pages: u64| {
        sys::vmar_map(vmar, vmo, 0, pages * PAGE, RW | vmar_flags::FIXED, addr)
    };
    c.status(
        "remapping over the whole range",
        fixed(base, 8),
        Status::InvalidArgs,
    );
    c.status(
        "remapping over the edge of the hole",
        fixed(base + PAGE, 2),
        Status::InvalidArgs,
    );
    c.expect(
        "the hole takes a FIXED mapping",
        fixed(base + 2 * PAGE, 2).is_ok(),
    );
    c.expect(
        "the hole mapping shows pages 0 and 1 of the VMO",
        first.bytes()[2 * 4096 + 7] == 0x10 && first.bytes()[3 * 4096 + 7] == 0x11,
    );
    c.expect(
        "an unmap that covers holes and regions",
        sys::vmar_unmap(vmar, base, 8 * PAGE).is_ok(),
    );
    c.expect("the range is free", fixed(base, 8).is_ok());
    let _ = sys::handle_close(vmo);
    c.expect(
        "a mapping keeps the VMO alive after its handle closes",
        second.bytes()[7] == 0x10 && second.bytes()[7 * 4096 + 7] == 0x17,
    );
    drop(second);
    drop(first);
}

/// Mapear e desmapear em laço: nenhuma promessa de tabela fica para trás.
fn map_loop(c: &mut Checks, cx: &Cx<'_>) {
    let vmar = cx.parent.vmar;
    let mut done = 0;
    for _ in 0..500 {
        let Ok(vmo) = sys::vmo_create(64 * PAGE) else {
            break;
        };
        if let Ok(mut map) = Mapping::new(vmar, vmo, 0, 64 * 4096_usize, RW) {
            for page in 0..64 {
                map.bytes_mut()[page * 4096] = 1;
            }
            done += 1;
        }
        let _ = sys::handle_close(vmo);
    }
    c.expect(
        "500 rounds of create, map, touch, unmap and close",
        done == 500,
    );
}

/// Criação de processo sem o direito, com argumento ruim, e o que o erro consome.
fn process_rights(c: &mut Checks, cx: &Cx<'_>) {
    let me = cx.parent.process;
    let pair = || sys::channel_create().ok();
    let (Some((keep, give)), Ok(weak)) = (pair(), sys::handle_duplicate(me, Rights::WAIT)) else {
        c.expect("a channel and a weak process handle", false);
        return;
    };
    c.status(
        "process_create without MANAGE_PROCESS",
        sys::process_create(weak, b"x", give),
        Status::AccessDenied,
    );
    c.status(
        "the bootstrap handle was consumed by the refused call",
        sys::handle_close(give),
        Status::BadHandle,
    );
    let Some((_, give)) = pair() else {
        return;
    };
    c.status(
        "process_create with a channel as the parent",
        sys::process_create(keep, b"x", give),
        Status::WrongType,
    );
    c.status(
        "the bootstrap was consumed by the wrong type",
        sys::handle_close(give),
        Status::BadHandle,
    );
    let Some((_, give)) = pair() else {
        return;
    };
    let Ok(no_transfer) = sys::handle_duplicate(give, Rights::READ.union(Rights::WRITE)) else {
        return;
    };
    c.status(
        "a bootstrap without TRANSFER",
        sys::process_create(me, b"x", no_transfer),
        Status::AccessDenied,
    );
    c.status(
        "the bootstrap without TRANSFER was consumed",
        sys::handle_close(no_transfer),
        Status::BadHandle,
    );
    process_args(c, me, give);
    let _ = sys::handle_close(keep);
}

/// Nome e ponteiro de saída: o que cada erro de argumento consome.
fn process_args(c: &mut Checks, me: Handle, give: Handle) {
    let pair = || sys::channel_create().ok();
    let long = [b'n'; 33];
    c.status(
        "a name of 33 bytes",
        sys::process_create(me, &long, give),
        Status::InvalidArgs,
    );
    c.status(
        "the bootstrap was consumed by the long name",
        sys::handle_close(give),
        Status::BadHandle,
    );
    let Some((_, give)) = pair() else {
        return;
    };
    let mut out = [0_u64; 2];
    let out_ptr = out.as_mut_ptr().addr() as u64;
    c.status(
        "a bad out pointer",
        raw(
            Syscall::ProcessCreate,
            [me.raw(), 0, 0, give.raw(), 0, UNMAPPED],
        ),
        Status::OutOfRange,
    );
    c.status(
        "the bootstrap was consumed by the bad out pointer",
        sys::handle_close(give),
        Status::BadHandle,
    );
    let Some((_, give)) = pair() else {
        return;
    };
    c.status(
        "the same handle as bootstrap and events",
        raw(
            Syscall::ProcessCreate,
            [me.raw(), 0, 0, give.raw(), give.raw(), out_ptr],
        ),
        Status::BadHandle,
    );
    c.expect("no refused call created anything", out == [0, 0]);
    c.status(
        "a forged parent",
        raw(Syscall::ProcessCreate, [0xBEEF_0001, 0, 0, 0, 0, out_ptr]),
        Status::BadHandle,
    );
}

/// `events` sem `WRITE` ou sem `TRANSFER`: o erro de direito consome os dois handles e não cria nada.
fn events_rights(c: &mut Checks, cx: &Cx<'_>) {
    let me = cx.parent.process;
    let cases = [
        ("events without WRITE", Rights::READ.union(Rights::TRANSFER)),
        ("events without TRANSFER", Rights::READ.union(Rights::WRITE)),
    ];
    for (what, rights) in cases {
        let (Some((keep, end)), Some((_, boot))) = (pair(), pair()) else {
            c.expect("two channels for the events case", false);
            return;
        };
        let Ok(weak) = sys::handle_duplicate(end, rights) else {
            c.expect("handle_duplicate of the events end", false);
            return;
        };
        let mut out = [0_u64; 2];
        let out_ptr = out.as_mut_ptr().addr() as u64;
        c.status(
            what,
            raw(
                Syscall::ProcessCreate,
                [me.raw(), 0, 0, boot.raw(), weak.raw(), out_ptr],
            ),
            Status::AccessDenied,
        );
        c.expect("the refused call created nothing", out == [0, 0]);
        c.status(
            "its bootstrap was consumed",
            sys::handle_close(boot),
            Status::BadHandle,
        );
        c.status(
            "its events handle was consumed",
            sys::handle_close(weak),
            Status::BadHandle,
        );
        c.expect(
            "the original events end is still ours",
            sys::handle_close(end).is_ok(),
        );
        let _ = sys::handle_close(keep);
    }
}

/// `events` que não é canal (console, VMO) e `events` igual ao próprio pai: `WrongType`, consumindo.
fn events_type(c: &mut Checks, cx: &Cx<'_>) {
    let me = cx.parent.process;
    let mut out = [0_u64; 2];
    let out_ptr = out.as_mut_ptr().addr() as u64;
    let Ok(console) = sys::handle_duplicate(cx.console, Rights::WRITE.union(Rights::TRANSFER))
    else {
        c.expect("a console handle to pass as events", false);
        return;
    };
    let Ok(vmo) = sys::vmo_create(PAGE) else {
        c.expect("a VMO to pass as events", false);
        return;
    };
    let Ok(movable) = sys::handle_duplicate(vmo, Rights::WRITE.union(Rights::TRANSFER)) else {
        c.expect("a VMO handle to pass as events", false);
        return;
    };
    let Some((_, boot)) = pair() else {
        c.expect("a channel for the type case", false);
        return;
    };
    c.status(
        "events as a console",
        raw(
            Syscall::ProcessCreate,
            [me.raw(), 0, 0, boot.raw(), console.raw(), out_ptr],
        ),
        Status::WrongType,
    );
    c.status(
        "the console handle was consumed",
        sys::handle_close(console),
        Status::BadHandle,
    );
    c.status(
        "the bootstrap was consumed with it",
        sys::handle_close(boot),
        Status::BadHandle,
    );
    let Some((_, boot)) = pair() else {
        c.expect("a channel for the VMO case", false);
        return;
    };
    c.status(
        "events as a VMO",
        raw(
            Syscall::ProcessCreate,
            [me.raw(), 0, 0, boot.raw(), movable.raw(), out_ptr],
        ),
        Status::WrongType,
    );
    c.status(
        "the VMO handle was consumed",
        sys::handle_close(movable),
        Status::BadHandle,
    );
    c.status(
        "the bootstrap was consumed with the VMO",
        sys::handle_close(boot),
        Status::BadHandle,
    );
    let Ok(parent_copy) = sys::handle_duplicate(me, Rights::MANAGE_PROCESS.union(Rights::TRANSFER))
    else {
        c.expect("a copy of the parent to pass as events", false);
        return;
    };
    let Some((_, boot)) = pair() else {
        c.expect("a channel for the parent case", false);
        return;
    };
    c.status(
        "events equal to the parent",
        raw(
            Syscall::ProcessCreate,
            [
                parent_copy.raw(),
                0,
                0,
                boot.raw(),
                parent_copy.raw(),
                out_ptr,
            ],
        ),
        Status::WrongType,
    );
    c.status(
        "the parent copy was consumed",
        sys::handle_close(parent_copy),
        Status::BadHandle,
    );
    c.status(
        "the bootstrap was consumed with the parent copy",
        sys::handle_close(boot),
        Status::BadHandle,
    );
    c.expect("no refused call created anything", out == [0, 0]);
    c.expect(
        "the parent itself is still ours",
        sys::handle_duplicate(me, Rights::WAIT).is_ok(),
    );
}

/// Um canal novo, com as duas pontas.
fn pair() -> Option<(Handle, Handle)> {
    sys::channel_create().ok()
}

/// Um filho com `events` como canal de eventos, com a mensagem `msg` (modo e argumento) e a thread
/// iniciada. A ponta de `events` sai do pai.
fn start_with_events(
    cx: &Cx<'_>,
    msg: [u8; 2],
    events: Handle,
) -> Result<(Child, Handle), spawn::Error> {
    let child = spawn::create_with_events(cx.parent, cx.child, events)?;
    child.send_boot(&[])?;
    sys::channel_write(child.channel, &msg, &[]).map_err(spawn::Error::Sys)?;
    let sp = child.stack()?;
    let thread = child.start(sp)?;
    Ok((child, thread))
}

/// O evento do fim, lido da ponta do pai: `None` se não chega em 5 s.
fn read_event(end: Handle) -> Option<ProcessEvent> {
    if !wait(end, signals::READABLE, 5 * SECOND) {
        return None;
    }
    let mut bytes = [0_u8; ProcessEvent::WIRE_LEN];
    let actual = sys::channel_read(end, &mut bytes, &mut []).ok()?;
    if actual.bytes != ProcessEvent::WIRE_LEN as u64 {
        return None;
    }
    ProcessEvent::from_bytes(&bytes)
}

/// O canal de eventos: a saída com código, a falta de escrita num endereço não mapeado, a morte por
/// `process_kill` e o pai que já fechou a ponta. Em todos, o evento vem antes do `PEER_CLOSED`.
fn events_channel(c: &mut Checks, cx: &Cx<'_>) {
    let Some((mine, theirs)) = pair() else {
        c.expect("a channel for the events", false);
        return;
    };
    match start_with_events(cx, [MODE_EXIT_CODE, 7], theirs) {
        Ok((child, thread)) => {
            c.expect(
                "a child that exits with 7 terminates",
                child.wait(now() + 5 * SECOND).is_ok(),
            );
            c.expect(
                "the event is already there when TERMINATED is seen",
                wait(mine, signals::READABLE, 0),
            );
            let event = read_event(mine);
            c.expect(
                "the parent reads an Exit event with code 7",
                event.is_some_and(|e| e.kind == event_kind::EXIT && e.code == 7),
            );
            c.expect(
                "after the event the parent sees PEER_CLOSED",
                wait(mine, signals::PEER_CLOSED, 5 * SECOND),
            );
            let _ = sys::handle_close(thread);
            child.release();
        }
        Err(_) => c.expect("a child with an events channel", false),
    }
    let _ = sys::handle_close(mine);
    let Some((mine, theirs)) = pair() else {
        c.expect("a channel for the fault event", false);
        return;
    };
    match start_with_events(cx, [MODE_OVERFLOW, 0], theirs) {
        Ok((child, thread)) => {
            c.expect(
                "a child that overflows its stack terminates",
                child.wait(now() + 5 * SECOND).is_ok(),
            );
            let event = read_event(mine);
            c.expect(
                "the overflow reads as a page fault with WRITE, at a user address",
                event.is_some_and(|e| {
                    e.kind == event_kind::FAULT
                        && e.fault_kind == fault_kind::PAGE_FAULT
                        && e.access & fault_access::WRITE != 0
                        && e.addr != 0
                        && e.addr < USER_TOP
                }),
            );
            c.expect(
                "the fault address is within a page of the stack pointer",
                event.is_some_and(|e| e.addr.abs_diff(e.sp) < PAGE),
            );
            c.expect(
                "the fault pc is a user address",
                event.is_some_and(|e| (USER_MIN..USER_TOP).contains(&e.pc)),
            );
            c.expect(
                "after the fault event the parent sees PEER_CLOSED",
                wait(mine, signals::PEER_CLOSED, 5 * SECOND),
            );
            let _ = sys::handle_close(thread);
            child.release();
        }
        Err(_) => c.expect("a child for the fault event", false),
    }
    let _ = sys::handle_close(mine);
    events_kill_and_gone(c, cx);
}

/// O evento da morte por `process_kill`, e o pai que fechou a ponta antes de o filho morrer.
fn events_kill_and_gone(c: &mut Checks, cx: &Cx<'_>) {
    let Some((mine, theirs)) = pair() else {
        c.expect("a channel for the kill event", false);
        return;
    };
    match start_with_events(cx, [MODE_BLOCK, 0], theirs) {
        Ok((child, thread)) => {
            c.expect(
                "process_kill of a blocked child",
                sys::process_kill(child.process).is_ok(),
            );
            c.expect(
                "the killed child terminates",
                child.wait(now() + 5 * SECOND).is_ok(),
            );
            let event = read_event(mine);
            c.expect(
                "a killed process reads an Exit event with the kill code",
                event.is_some_and(|e| e.kind == event_kind::EXIT && e.code == PROCESS_KILLED_CODE),
            );
            let _ = sys::handle_close(thread);
            child.release();
        }
        Err(_) => c.expect("a child for the kill event", false),
    }
    let _ = sys::handle_close(mine);
    let Some((mine, theirs)) = pair() else {
        c.expect("a channel for the parent-gone case", false);
        return;
    };
    let _ = sys::handle_close(mine);
    match start_with_events(cx, [MODE_EXIT_CODE, 3], theirs) {
        Ok((child, thread)) => {
            c.expect(
                "a child whose parent closed the events end still terminates",
                child.wait(now() + 5 * SECOND).is_ok(),
            );
            let _ = sys::handle_close(thread);
            child.release();
        }
        Err(_) => c.expect("a child with the events end closed by the parent", false),
    }
}

/// Thread e kill sem o direito, argumentos ruins, e a vida de um processo que nunca rodou.
fn thread_rules(c: &mut Checks, cx: &Cx<'_>) {
    let Some((_, give)) = sys::channel_create().ok() else {
        return;
    };
    let Ok((process, vmar)) = sys::process_create(cx.parent.process, b"idle", give) else {
        c.expect("process_create", false);
        return;
    };
    let (Ok(manage), Ok(wait_only)) = (
        sys::handle_duplicate(process, Rights::MANAGE_THREAD.union(Rights::DESTROY)),
        sys::handle_duplicate(process, Rights::WAIT.union(Rights::MANAGE_PROCESS)),
    ) else {
        return;
    };
    let code = 0x40_0000;
    c.status(
        "thread_create without MANAGE_THREAD",
        sys::thread_create(wait_only, code, 0x7000_0000, 0, 0),
        Status::AccessDenied,
    );
    c.status(
        "process_kill without DESTROY",
        sys::process_kill(wait_only),
        Status::AccessDenied,
    );
    for (what, entry, sp) in [
        ("an entry in the null zone", 0x1000, 0x7000_0000),
        ("an entry at zero", 0, 0x7000_0000),
        ("an entry at the user top", USER_TOP, 0x7000_0000),
        ("an entry in the kernel half", KERNEL, 0x7000_0000),
        ("a stack above the top", code, USER_TOP + PAGE),
        ("an unaligned stack", code, 0x7000_0008),
        ("a stack in the kernel half", code, KERNEL),
    ] {
        c.status(
            what,
            sys::thread_create(manage, entry, sp, 0, 0),
            Status::InvalidArgs,
        );
    }
    c.status(
        "thread_create with a VMAR",
        sys::thread_create(vmar, code, 0x7000_0000, 0, 0),
        Status::WrongType,
    );
    c.expect(
        "an unstarted process has not terminated",
        sys::object_wait_one(process, signals::TERMINATED, 0) == Err(Status::TimedOut),
    );
    c.expect(
        "process_kill of a process that never ran",
        sys::process_kill(manage).is_ok(),
    );
    c.expect(
        "the killed process is terminated at once",
        wait(process, signals::TERMINATED, 0),
    );
    c.status(
        "process_kill twice",
        sys::process_kill(manage),
        Status::BadState,
    );
    c.status(
        "thread_create in a dead process",
        sys::thread_create(manage, code, 0x7000_0000, 0, 0),
        Status::BadState,
    );
    c.status(
        "vmar_map into a dead process",
        sys::vmar_map(vmar, cx.initramfs, 0, PAGE, vmar_flags::READ, 0),
        Status::BadState,
    );
    for handle in [process, vmar, manage, wait_only] {
        let _ = sys::handle_close(handle);
    }
}

/// Uma thread a mais com `arg0` (`WORKER_SPIN` ou `WORKER_EXIT`) e a pilha dela.
fn worker(child: &Child, arg0: u64) -> Option<Handle> {
    let sp = child.stack().ok()?;
    sys::thread_create(child.process, child.entry(), sp, arg0, 0).ok()
}

/// A thread que sai, o processo que acaba com a última, e a morte de threads de uma vez.
fn threads_and_kill(c: &mut Checks, cx: &Cx<'_>) {
    if let Ok(child) = start(cx, Some(MODE_BLOCK), false) {
        let thread = child.stack().and_then(|sp| child.start(sp));
        let exiting = worker(&child, WORKER_EXIT);
        c.expect(
            "a worker that calls thread_exit terminates",
            exiting.is_some_and(|thread| wait(thread, signals::TERMINATED, 5 * SECOND)),
        );
        c.expect(
            "the process outlives a thread that exited",
            sys::object_wait_one(child.process, signals::TERMINATED, now() + 20 * MS)
                == Err(Status::TimedOut),
        );
        let started = now();
        c.expect(
            "process_kill of a process blocked in the kernel",
            sys::process_kill(child.process).is_ok(),
        );
        c.expect(
            "the blocked thread died and the process terminated",
            child.wait(now() + 5 * SECOND).is_ok(),
        );
        c.expect(
            "the thread handle sees TERMINATED too",
            thread.is_ok_and(|thread| wait(thread, signals::TERMINATED, 0)),
        );
        println!(
            "procs: kill of a process blocked in the kernel took {} us",
            (now() - started) / 1000
        );
        child.release();
    } else {
        c.expect("a child in block mode", false);
    }
    if let Ok(child) = start(cx, Some(MODE_BLOCK), false) {
        let only = worker(&child, WORKER_EXIT);
        c.expect(
            "a process whose only thread exits terminates",
            only.is_some() && child.wait(now() + 5 * SECOND).is_ok(),
        );
        child.release();
    }
    if let Ok(child) = start(cx, Some(MODE_SPIN), false) {
        let first = child.stack().and_then(|sp| child.start(sp));
        let mut spinners = [Handle::INVALID; 3];
        let mut started_workers = 0;
        for slot in &mut spinners {
            if let Some(thread) = worker(&child, WORKER_SPIN) {
                *slot = thread;
                started_workers += 1;
            }
        }
        c.expect(
            "the main thread and three spinning workers started",
            started_workers == 3 && first.is_ok(),
        );
        sleep(30 * MS);
        c.expect(
            "four threads spin and the process has not terminated",
            sys::object_wait_one(child.process, signals::TERMINATED, 0) == Err(Status::TimedOut),
        );
        let started = now();
        c.expect(
            "process_kill with threads spinning in user mode",
            sys::process_kill(child.process).is_ok(),
        );
        c.expect(
            "every thread stopped and the process terminated",
            child.wait(now() + 5 * SECOND).is_ok(),
        );
        println!(
            "procs: kill of a process with 4 spinning threads took {} us",
            (now() - started) / 1000
        );
        c.status(
            "process_kill again",
            sys::process_kill(child.process),
            Status::BadState,
        );
        c.expect(
            "the worker handles see TERMINATED",
            spinners
                .iter()
                .all(|&thread| wait(thread, signals::TERMINATED, 0)),
        );
        child.release();
    } else {
        c.expect("a child in spin mode", false);
    }
    deaths(c, cx);
}

/// O processo que se mata e o estouro de pilha numa thread com outra girando.
fn deaths(c: &mut Checks, cx: &Cx<'_>) {
    if let Ok(child) = start(cx, Some(MODE_SELF_KILL), true) {
        let started = child.stack().and_then(|sp| child.start(sp));
        c.expect(
            "a process that kills itself terminates and the call does not come back",
            started.is_ok() && child.wait(now() + 5 * SECOND).is_ok(),
        );
        if let Ok(thread) = started {
            let _ = sys::handle_close(thread);
        }
        child.release();
    } else {
        c.expect("a child in self-kill mode", false);
    }
    if let Ok(child) = start(cx, Some(MODE_OVERFLOW), false) {
        let spinner = worker(&child, WORKER_SPIN);
        let main = child.stack().and_then(|sp| child.start(sp));
        c.expect(
            "a stack overflow in one thread kills the process and stops the spinning one",
            main.is_ok() && spinner.is_some() && child.wait(now() + 5 * SECOND).is_ok(),
        );
        child.release();
    } else {
        c.expect("a child in overflow mode", false);
    }
}

/// Esgotar o teto em bytes: mensagens em fila, VMOs pequenos e processos que nunca rodam.
fn byte_cap(c: &mut Checks, cx: &Cx<'_>) {
    let mut ends = [Handle::INVALID; 256];
    let (mut pairs, mut written, mut refused) = (0, 0_u64, None);
    let message = [0xA5_u8; 1024];
    'fill: while pairs < 128 {
        let Ok((a, b)) = sys::channel_create() else {
            refused = Some("channel_create");
            break;
        };
        ends[pairs * 2] = a;
        ends[pairs * 2 + 1] = b;
        pairs += 1;
        for end in [a, b] {
            for _ in 0..16 {
                match sys::channel_write(end, &message, &[]) {
                    Ok(()) => written += 1024,
                    Err(Status::NoMemory) => {
                        refused = Some("channel_write");
                        break 'fill;
                    }
                    Err(_) => break 'fill,
                }
            }
        }
    }
    println!(
        "procs: the byte cap stopped {} at {pairs} pairs and {written} message bytes",
        refused.unwrap_or("nothing")
    );
    c.expect(
        "the byte cap answers NoMemory before 128 pairs",
        refused.is_some(),
    );
    c.expect(
        "the cap bounds what a process queues in the kernel",
        written > 64 * 1024 && written <= 512 * 1024,
    );
    for end in ends {
        let _ = sys::handle_close(end);
    }
    let (a, b) = sys::channel_create().unwrap_or((Handle::INVALID, Handle::INVALID));
    c.expect(
        "closing every end gives the bytes back: a message fits again",
        a != Handle::INVALID && sys::channel_write(a, &message, &[]).is_ok(),
    );
    let _ = (sys::handle_close(a), sys::handle_close(b));
    cap_objects(c, cx);
}

/// O mesmo teto com VMOs pequenos e com processos que nunca rodam.
fn cap_objects(c: &mut Checks, cx: &Cx<'_>) {
    let mut vmos = 0_usize;
    let mut handles = [Handle::INVALID; 2000];
    let status = loop {
        match sys::vmo_create(PAGE) {
            Ok(vmo) => {
                handles[vmos] = vmo;
                vmos += 1;
                if vmos == handles.len() {
                    break None;
                }
            }
            Err(status) => break Some(status),
        }
    };
    println!("procs: the byte cap stopped vmo_create at {vmos} one-page VMOs");
    c.expect(
        "the byte cap stops vmo_create with NoMemory",
        status == Some(Status::NoMemory) && vmos > 500,
    );
    for handle in handles {
        let _ = sys::handle_close(handle);
    }
    c.expect(
        "closing the VMOs gives the cap back",
        sys::vmo_create(PAGE).is_ok(),
    );
    let mut made = 0_usize;
    let mut processes = [Handle::INVALID; 128];
    let status = loop {
        let Some((_, give)) = sys::channel_create().ok() else {
            break Some(Status::NoMemory);
        };
        match sys::process_create(cx.parent.process, b"cap", give) {
            Ok((process, vmar)) => {
                processes[made] = process;
                let _ = sys::handle_close(vmar);
                made += 1;
                if made == processes.len() {
                    break None;
                }
            }
            Err(status) => break Some(status),
        }
    };
    println!("procs: the byte cap stopped process_create at {made} processes");
    c.expect(
        "the byte cap stops process_create with NoMemory",
        status == Some(Status::NoMemory) && made > 20,
    );
    for handle in processes {
        let _ = sys::handle_close(handle);
    }
}

/// Um processo por rodada: criar, mapear o programa, iniciar, esperar `TERMINATED` (ou matar logo) e
/// soltar tudo. Devolve os microssegundos por processo.
fn lifecycle_loop(c: &mut Checks, cx: &Cx<'_>, rounds: u32) -> u64 {
    let started = now();
    let mut good = 0;
    for round in 0..rounds {
        let Ok(child) = start(cx, Some(MODE_QUIET), false) else {
            break;
        };
        let Ok(thread) = child.stack().and_then(|sp| child.start(sp)) else {
            child.abandon();
            break;
        };
        if round % 2 == 1 {
            let _ = sys::process_kill(child.process);
        }
        good += u32::from(child.wait(now() + 5 * SECOND).is_ok());
        let _ = sys::handle_close(thread);
        child.release();
    }
    c.expect("every process of the loop terminated", good == rounds);
    (now() - started) / 1000 / u64::from(rounds.max(1))
}

/// O eco entre dois processos por canal, com `object_wait_one`: o análogo do `lat_ctx`. Cada ida e
/// volta são duas trocas de processo.
fn echo(c: &mut Checks, cx: &Cx<'_>, rounds: u8) -> u64 {
    let Ok(child) = spawn::create(cx.parent, cx.child) else {
        c.expect("a child for the echo", false);
        return 0;
    };
    let thread = child.send_boot(&[]).ok().and_then(|()| {
        sys::channel_write(child.channel, &[MODE_ECHO, rounds], &[]).ok()?;
        child.stack().and_then(|sp| child.start(sp)).ok()
    });
    c.expect("the echo child started", thread.is_some());
    let total = u32::from(rounds) * 16;
    let started = now();
    let mut good = 0;
    for round in 0..total {
        let token = [u8::try_from(round % 256).unwrap_or(0); 4];
        if sys::channel_write(child.channel, &token, &[]).is_err()
            || !wait(child.channel, signals::READABLE, 5 * SECOND)
        {
            break;
        }
        let mut back = [0_u8; 4];
        if sys::channel_read(child.channel, &mut back, &mut []).is_ok() && back == token {
            good += 1;
        }
    }
    let elapsed = now() - started;
    c.expect(
        "every token came back from the other process",
        good == total,
    );
    let _ = child.wait(now() + 5 * SECOND);
    if let Some(thread) = thread {
        let _ = sys::handle_close(thread);
    }
    child.release();
    elapsed / u64::from(total.max(1))
}

/// Solta o filho bloqueado que o ponto `B` mediu.
fn release_blocked(c: &mut Checks, blocked: Result<(Child, Handle), spawn::Error>) {
    if let Ok((child, thread)) = blocked {
        let _ = sys::process_kill(child.process);
        let _ = child.wait(now() + 5 * SECOND);
        let _ = sys::handle_close(thread);
        child.release();
    } else {
        c.expect("a blocked child to measure", false);
    }
}

/// Tudo o que o trecho medido faz, uma vez antes de `A`: o heap do kernel, as áreas de FPU dos slots
/// e as páginas do próprio programa já cresceram quando a medida começa.
fn warm_up(c: &mut Checks, cx: &Cx<'_>) {
    let mut crowd = [const { None }; 8];
    for slot in &mut crowd {
        *slot = start(cx, Some(MODE_BLOCK), false)
            .and_then(|child| {
                let thread = child.stack().and_then(|sp| child.start(sp))?;
                Ok((child, thread))
            })
            .ok();
    }
    sleep(MS);
    for blocked in crowd {
        release_blocked(c, blocked.ok_or(spawn::Error::Stack));
    }
    let _ = lifecycle_loop(c, cx, 8);
    let _ = echo(c, cx, 2);
}

fn main(env: &Env) -> i64 {
    let Some(boot) = boot::attach_console(env) else {
        return 99;
    };
    let Some(boot_channel) = env.boot_channel() else {
        return 98;
    };
    let Boot {
        console: Some(console),
        initramfs: Some(initramfs),
        ..
    } = boot
    else {
        println!("procs: FAILED the console and the initramfs in the boot message");
        return 97;
    };
    let Some(parent) = Parent::from_boot(&boot) else {
        return 96;
    };
    let Ok(archive) = Archive::open(parent.vmar, initramfs) else {
        println!("procs: FAILED mapping the initramfs");
        return 95;
    };
    let Some(child) = archive.file(b"child") else {
        println!("procs: FAILED the child program in the initramfs");
        return 94;
    };
    let cx = Cx {
        parent,
        console,
        initramfs,
        child,
        boot_channel,
    };
    let mut c = Checks {
        ran: 0,
        failed: 0,
        first: None,
    };
    vmo_vmar(&mut c, &cx);
    sharing_and_partial_unmap(&mut c, &cx);
    process_rights(&mut c, &cx);
    events_rights(&mut c, &cx);
    events_type(&mut c, &cx);
    events_channel(&mut c, &cx);
    thread_rules(&mut c, &cx);
    threads_and_kill(&mut c, &cx);
    byte_cap(&mut c, &cx);
    map_loop(&mut c, &cx);
    warm_up(&mut c, &cx);
    sleep(30 * MS);
    sync(&cx, b"A");
    let blocked = start(&cx, Some(MODE_BLOCK), false).and_then(|child| {
        let thread = child.stack().and_then(|sp| child.start(sp))?;
        Ok((child, thread))
    });
    sleep(20 * MS);
    sync(&cx, b"B");
    release_blocked(&mut c, blocked);
    let per_process = lifecycle_loop(&mut c, &cx, LOOPS);
    println!(
        "procs: create, map, start, run, exit and wait took {per_process} us per process ({LOOPS} processes)"
    );
    let per_round = echo(&mut c, &cx, u8::try_from(ECHOES / 16).unwrap_or(1));
    println!("procs: echo between two processes took {per_round} ns per round trip");
    sleep(30 * MS);
    sync(&cx, b"C");
    if c.failed == 0 {
        println!("procs: ok ({} assertions)", c.ran);
        0
    } else {
        println!("procs: FAILED {} of {} assertions", c.failed, c.ran);
        i64::from(c.failed)
    }
}
