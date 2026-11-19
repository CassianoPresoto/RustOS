//! As syscalls da ABI nativa (4a) como funções seguras: o slice garante o ponteiro e o tamanho que
//! o kernel recebe, e o status vira `Result`.

use rustos_hal::abi::{
    Handle, ReadActual, Rights, Status, Syscall, SyscallArgs, SyscallReturn, USER_HANDLE_ARRAY_MAX,
};

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

/// `debug_write`: escreve `bytes` no console `console` (um handle de console com `WRITE`), no
/// máximo `DEBUG_WRITE_MAX`. Devolve os bytes escritos.
#[allow(
    clippy::cast_possible_truncation,
    reason = "o usuário só existe em 64 bits"
)]
pub fn debug_write(console: Handle, bytes: &[u8]) -> Result<usize> {
    let args = [
        console.raw(),
        bytes.as_ptr().addr() as u64,
        bytes.len() as u64,
        0,
        0,
        0,
    ];
    checked(call(Syscall::DebugWrite, args)).map(|written| written as usize)
}

/// Um valor que o kernel devolveu como handle: nunca é zero.
fn handle_of(raw: u64) -> Result<Handle> {
    Handle::from_raw(raw).ok_or(Status::NotSupported)
}

/// `handle_close`.
pub fn handle_close(handle: Handle) -> Result<()> {
    checked(call(Syscall::HandleClose, [handle.raw(), 0, 0, 0, 0, 0])).map(drop)
}

/// `handle_duplicate`: um handle novo para o mesmo objeto, com `rights` (um subconjunto).
pub fn handle_duplicate(handle: Handle, rights: Rights) -> Result<Handle> {
    let args = [handle.raw(), u64::from(rights.bits()), 0, 0, 0, 0];
    checked(call(Syscall::HandleDuplicate, args)).and_then(handle_of)
}

/// `handle_replace`: o velho deixa de valer sempre que existe, e o novo tem `rights`.
pub fn handle_replace(handle: Handle, rights: Rights) -> Result<Handle> {
    let args = [handle.raw(), u64::from(rights.bits()), 0, 0, 0, 0];
    checked(call(Syscall::HandleReplace, args)).and_then(handle_of)
}

/// `object_wait_one`: espera um dos `signals` até o instante `deadline_ns` do relógio monotônico
/// (`u64::MAX`: sem fim). Devolve todos os sinais que valem.
#[allow(
    clippy::cast_possible_truncation,
    reason = "os sinais cabem em 32 bits"
)]
pub fn object_wait_one(handle: Handle, signals: u32, deadline_ns: u64) -> Result<u32> {
    let args = [handle.raw(), u64::from(signals), deadline_ns, 0, 0, 0];
    checked(call(Syscall::ObjectWaitOne, args)).map(|observed| observed as u32)
}

/// `channel_create`: as duas pontas de um canal novo.
pub fn channel_create() -> Result<(Handle, Handle)> {
    let mut out = [0_u64; 2];
    let args = [0, out.as_mut_ptr().addr() as u64, 0, 0, 0, 0];
    checked(call(Syscall::ChannelCreate, args))?;
    Ok((handle_of(out[0])?, handle_of(out[1])?))
}

/// `channel_write`: `bytes` e os `handles`, que saem da tabela (o kernel os move ou os fecha).
pub fn channel_write(channel: Handle, bytes: &[u8], handles: &[Handle]) -> Result<()> {
    let args = [
        channel.raw(),
        bytes.as_ptr().addr() as u64,
        bytes.len() as u64,
        handles.as_ptr().addr() as u64,
        handles.len() as u64,
        0,
    ];
    checked(call(Syscall::ChannelWrite, args)).map(drop)
}

/// O que um `channel_read` que falhou diz da mensagem: com `BufferTooSmall`, o que ela pede.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadError {
    pub status: Status,
    pub needs: ReadActual,
}

/// `channel_read`: lê a mensagem da frente para `bytes` e `handles` e devolve os tamanhos.
pub fn channel_read(
    channel: Handle,
    bytes: &mut [u8],
    handles: &mut [Handle],
) -> core::result::Result<ReadActual, ReadError> {
    let mut actual = ReadActual::default();
    let args = [
        channel.raw(),
        bytes.as_mut_ptr().addr() as u64,
        bytes.len() as u64,
        handles.as_mut_ptr().addr() as u64,
        handles.len().min(USER_HANDLE_ARRAY_MAX) as u64,
        (&raw mut actual).addr() as u64,
    ];
    match checked(call(Syscall::ChannelRead, args)) {
        Ok(_) => Ok(actual),
        Err(status) => Err(ReadError {
            status,
            needs: actual,
        }),
    }
}

/// Chamadas com valores crus, para o programa que testa o que o kernel recusa: um handle forjado, um
/// ponteiro ruim, um tamanho acima do limite. O kernel valida tudo, mas um ponteiro válido do próprio
/// programa é escrito de verdade: só o `user/tests/*` liga esta feature (o `check-layers` confere).
#[cfg(feature = "raw-syscalls")]
pub mod raw {
    use super::{Result, Syscall, call, checked};

    /// Uma chamada com os argumentos como vierem.
    /// # Safety
    /// Todo ponteiro que o kernel escreve é de um buffer vivo do programa com o tamanho passado,
    /// ou um endereço que o kernel recusa (nulo, não mapeado, do kernel).
    pub unsafe fn syscall(number: Syscall, args: [u64; 6]) -> Result<u64> {
        checked(call(number, args))
    }
}

/// `vmo_create`: um VMO anônimo e preguiçoso de `size` bytes (múltiplo de página).
pub fn vmo_create(size: u64) -> Result<Handle> {
    checked(call(Syscall::VmoCreate, [size, 0, 0, 0, 0, 0])).and_then(handle_of)
}

/// `vmar_map`: mapeia `len` bytes do VMO a partir de `vmo_off` com as permissões de `flags`
/// (`vmar_flags`). Com `FIXED` o endereço é `vmar_off`; sem ele o kernel escolhe. Devolve o endereço.
pub fn vmar_map(
    vmar: Handle,
    vmo: Handle,
    vmo_off: u64,
    len: u64,
    flags: u32,
    vmar_off: u64,
) -> Result<u64> {
    let args = [
        vmar.raw(),
        vmo.raw(),
        vmo_off,
        len,
        u64::from(flags),
        vmar_off,
    ];
    checked(call(Syscall::VmarMap, args))
}

/// `vmar_unmap`: solta `[addr, addr + len)` do VMAR.
pub fn vmar_unmap(vmar: Handle, addr: u64, len: u64) -> Result<()> {
    checked(call(Syscall::VmarUnmap, [vmar.raw(), addr, len, 0, 0, 0])).map(drop)
}

/// `process_create`: um processo vazio sob `parent` (com `MANAGE_PROCESS`); `bootstrap` sai da tabela
/// e vira o handle 1 dele. Devolve o processo e o VMAR raiz.
pub fn process_create(parent: Handle, name: &[u8], bootstrap: Handle) -> Result<(Handle, Handle)> {
    let mut out = [0_u64; 2];
    let args = [
        parent.raw(),
        name.as_ptr().addr() as u64,
        name.len() as u64,
        bootstrap.raw(),
        0,
        out.as_mut_ptr().addr() as u64,
    ];
    checked(call(Syscall::ProcessCreate, args))?;
    Ok((handle_of(out[0])?, handle_of(out[1])?))
}

/// `thread_create`: cria e inicia uma thread em `process` (a primeira inicia o processo); `arg0` e
/// `arg1` chegam em `rdi`/`rsi` (`x0`/`x1`).
pub fn thread_create(process: Handle, entry: u64, sp: u64, arg0: u64, arg1: u64) -> Result<Handle> {
    let args = [process.raw(), entry, sp, arg0, arg1, 0];
    checked(call(Syscall::ThreadCreate, args)).and_then(handle_of)
}

/// `thread_exit`: a thread sai; o processo acaba com a última. Não volta.
pub fn thread_exit() -> ! {
    let _ = call(Syscall::ThreadExit, [0; 6]);
    loop {
        core::hint::spin_loop();
    }
}

/// `process_kill`: o processo morre (`DESTROY`); a thread que o chama também, se for dele.
pub fn process_kill(process: Handle) -> Result<()> {
    checked(call(Syscall::ProcessKill, [process.raw(), 0, 0, 0, 0, 0])).map(drop)
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
