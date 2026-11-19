//! O despacho das chamadas de sistema da ABI nativa (`hal::abi`, SDD 17). Na fatia 4a só atende
//! `nop`, `clock_get_monotonic`, `random_get`, `process_exit` e a `debug_write` provisória, sem
//! handle; o resto devolve `NotSupported` e o processo segue. Roda mascarado, na pilha da tarefa.

use core::fmt::Write as _;
use rustos_hal::abi::{
    DEBUG_WRITE_MAX, RANDOM_GET_MAX, Status, Syscall, SyscallArgs, SyscallReturn, debug_sanitize,
};

use rustos_mm::UserRange;

use crate::{console, platform, process, random};

#[allow(
    clippy::cast_possible_truncation,
    reason = "o kernel só existe em 64 bits"
)]
const DEBUG_BYTES: usize = DEBUG_WRITE_MAX as usize;
#[allow(
    clippy::cast_possible_truncation,
    reason = "o kernel só existe em 64 bits"
)]
const RANDOM_BYTES: usize = RANDOM_GET_MAX as usize;

#[allow(
    clippy::cast_possible_truncation,
    reason = "o kernel só existe em 64 bits"
)]
const fn slot(number: u64) -> usize {
    number as usize
}

/// Entradas da tabela de despacho: o maior número da ABI mais um.
const TABLE_LEN: usize = {
    let mut max = 0;
    let mut index = 0;
    while index < Syscall::ALL.len() {
        let number = slot(Syscall::ALL[index].raw());
        if number > max {
            max = number;
        }
        index += 1;
    }
    max + 1
};

const fn build_table() -> [Option<Syscall>; TABLE_LEN] {
    let mut table = [None; TABLE_LEN];
    let mut index = 0;
    while index < Syscall::ALL.len() {
        table[slot(Syscall::ALL[index].raw())] = Some(Syscall::ALL[index]);
        index += 1;
    }
    table
}

/// O número da chamada indexa a tabela direto; o vazio é número que a ABI não define.
static TABLE: [Option<Syscall>; TABLE_LEN] = build_table();

/// A tabela e `Syscall::from_raw` dão a mesma resposta para todo número até o fim da tabela.
const _: () = {
    let table = build_table();
    let mut number = 0;
    while number < TABLE_LEN {
        match (Syscall::from_raw(number as u64), table[number]) {
            (None, None) => {}
            (Some(expected), Some(found)) => assert!(expected.raw() == found.raw()),
            _ => panic!("a tabela de despacho e Syscall::from_raw divergem"),
        }
        number += 1;
    }
};

/// A chamada do número, ou `None`; o índice passa pelo clamp do Spectre v1 antes da tabela.
#[inline]
fn lookup(number: u64) -> Option<Syscall> {
    if number >= TABLE_LEN as u64 {
        return None;
    }
    TABLE[platform::array_index_nospec(slot(number), TABLE_LEN)]
}

/// `Handlers::syscall`: o número e os seis argumentos vindos do usuário.
pub fn dispatch(args: &SyscallArgs) -> SyscallReturn {
    process::note_first_syscall();
    let Some(call) = lookup(args.number) else {
        return SyscallReturn::err(Status::NotSupported);
    };
    let [a0, a1, ..] = args.args;
    match call {
        Syscall::Nop => {
            #[cfg(feature = "selftest")]
            process::on_nop();
            SyscallReturn::ok(0)
        }
        Syscall::ProcessExit => process::exit(a0.cast_signed()),
        Syscall::DebugWrite => debug_write(a0, a1),
        Syscall::ClockGetMonotonic => {
            platform::now_ns().map_or(SyscallReturn::err(Status::NotSupported), SyscallReturn::ok)
        }
        Syscall::RandomGet => random_get(a0, a1),
        _ => SyscallReturn::err(Status::NotSupported),
    }
}

/// Por que a cópia de usuário falhou: sem frame para a página preguiçosa, `NoMemory`; senão a faixa
/// não está mapeada.
fn copy_failure() -> Status {
    if process::copy_ran_out_of_memory() {
        Status::NoMemory
    } else {
        Status::OutOfRange
    }
}

/// `debug_write(ptr, len)`: escreve no console até `DEBUG_WRITE_MAX` bytes, sem ESC nem controle
/// (menos `\n` e `\t`), cada linha com o prefixo do processo. Devolve os bytes escritos.
#[inline(never)]
fn debug_write(ptr: u64, len: u64) -> SyscallReturn {
    if len == 0 {
        return SyscallReturn::err(Status::InvalidArgs);
    }
    let len = len.min(DEBUG_WRITE_MAX);
    let Ok(range) = UserRange::new(ptr, len) else {
        return SyscallReturn::err(Status::OutOfRange);
    };
    let mut buffer = [0_u8; DEBUG_BYTES];
    let bytes = &mut buffer[..usize::try_from(len).unwrap_or(0)];
    if !platform::copy_from_user(range, bytes) {
        return SyscallReturn::err(copy_failure());
    }
    console::print(format_args!("{}", Filtered(bytes)));
    SyscallReturn::ok(len)
}

/// Os bytes do usuário como texto que o console aceita (`hal::abi::debug_sanitize`): só ASCII
/// imprimível, cada linha com `[user] ` na frente e a saída sempre terminada em `\n`, de modo
/// que o usuário não imite uma linha `[rustos]` do kernel.
struct Filtered<'a>(&'a [u8]);

impl core::fmt::Display for Filtered<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut result = Ok(());
        debug_sanitize(self.0, "[user] ", |ch| {
            if result.is_ok() {
                result = f.write_char(ch);
            }
        });
        result
    }
}

/// `random_get(ptr, len)`: `len` de 1 a `RANDOM_GET_MAX` bytes do gerador do kernel.
#[inline(never)]
fn random_get(ptr: u64, len: u64) -> SyscallReturn {
    if len == 0 || len > RANDOM_GET_MAX {
        return SyscallReturn::err(Status::InvalidArgs);
    }
    let Ok(range) = UserRange::new(ptr, len) else {
        return SyscallReturn::err(Status::OutOfRange);
    };
    let count = usize::try_from(len).unwrap_or(0);
    let mut buffer = [0_u8; RANDOM_BYTES];
    let mut filled = 0;
    while filled < count {
        let Some(word) = random::next_u64() else {
            rustos_crypto::wipe(&mut buffer);
            return SyscallReturn::err(Status::NotSupported);
        };
        let take = (count - filled).min(8);
        buffer[filled..filled + take].copy_from_slice(&word.to_le_bytes()[..take]);
        filled += take;
    }
    let bytes = &buffer[..usize::try_from(len).unwrap_or(0)];
    let copied = platform::copy_to_user(range, bytes);
    rustos_crypto::wipe(&mut buffer);
    if copied {
        SyscallReturn::ok(0)
    } else {
        SyscallReturn::err(copy_failure())
    }
}
