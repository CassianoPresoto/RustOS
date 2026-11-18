//! As syscalls da ABI nativa (4a) como funções seguras: o slice garante o ponteiro e o tamanho que
//! o kernel recebe, e o status vira `Result`.

use rustos_hal::abi::{Status, Syscall, SyscallArgs, SyscallReturn};

#[cfg(target_arch = "aarch64")]
use rustos_arch_aarch64_user::syscall;
#[cfg(target_arch = "x86_64")]
use rustos_arch_x86_64_user::syscall;

/// O resultado de uma chamada: o valor devolvido ou o erro. Um status que a ABI não define vira
/// `NotSupported`.
pub type Result<T> = core::result::Result<T, Status>;

fn call(number: Syscall, args: [u64; 6]) -> SyscallReturn {
    let request = SyscallArgs {
        number: number.raw(),
        args,
    };
    // SAFETY: cada função deste módulo põe em `args` só ponteiros de slices vivos, com o tamanho
    // certo, ou valores que o kernel não desreferencia.
    unsafe { syscall(&request) }
}

fn checked(returned: SyscallReturn) -> Result<u64> {
    match Status::from_raw(returned.status) {
        Some(Status::Ok) => Ok(returned.value),
        Some(error) => Err(error),
        None => Err(Status::NotSupported),
    }
}

/// `nop`: só entra e sai do kernel.
pub fn nop() -> Result<()> {
    checked(call(Syscall::Nop, [0; 6])).map(drop)
}

/// `debug_write`: escreve `bytes` no console, no máximo `DEBUG_WRITE_MAX`. Devolve os bytes
/// escritos.
pub fn debug_write(bytes: &[u8]) -> Result<usize> {
    let args = [bytes.as_ptr().addr() as u64, bytes.len() as u64, 0, 0, 0, 0];
    checked(call(Syscall::DebugWrite, args)).map(|written| written as usize)
}

/// `clock_get_monotonic`: nanossegundos desde o boot.
pub fn clock_get_monotonic() -> Result<u64> {
    checked(call(Syscall::ClockGetMonotonic, [0; 6]))
}

/// `random_get`: enche `buffer` (de 1 a `RANDOM_GET_MAX` bytes) com bytes do gerador do kernel.
pub fn random_get(buffer: &mut [u8]) -> Result<()> {
    let args = [
        buffer.as_mut_ptr().addr() as u64,
        buffer.len() as u64,
        0,
        0,
        0,
        0,
    ];
    checked(call(Syscall::RandomGet, args)).map(drop)
}

/// `process_exit`: encerra o processo com `code`. Não volta.
pub fn process_exit(code: i64) -> ! {
    let _ = call(Syscall::ProcessExit, [code.cast_unsigned(), 0, 0, 0, 0, 0]);
    loop {
        core::hint::spin_loop();
    }
}
