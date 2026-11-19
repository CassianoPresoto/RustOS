//! O contrato entre o kernel e o programa de usuário: números de syscall, `Status`, direitos,
//! layout do espaço de usuário, `ProcessEvent` e a mensagem de boot (SDD 17, "ABI nativa mínima").
//! É a primeira porta de entrada do hexágono: mudar um valor aqui muda o contrato com o mundo.

use core::mem::size_of;

pub use crate::paging::PAGE_SIZE;

/// Primeiro endereço mapeável: abaixo dele fica a faixa nula, que nada mapeia.
pub const USER_MIN: u64 = 0x1_0000;
/// Fim exclusivo do espaço de usuário, igual nas duas ISAs; a última página nunca mapeia.
pub const USER_TOP: u64 = 0x0000_7FFF_FFFF_F000;
/// Onde o linker do usuário põe a imagem de um `ET_EXEC`.
pub const USER_IMAGE_BASE: u64 = 0x40_0000;
/// Teto da soma dos `p_memsz` dos `PT_LOAD` de um executável (os `Limits` do `core/elf`).
pub const EXEC_MAX_LOADS_BYTES: u64 = 64 << 20;

/// Bytes de um `debug_write`; o excedente é descartado (o lock do console mascara IRQ).
pub const DEBUG_WRITE_MAX: u64 = 512;
/// Bytes de um `random_get`.
pub const RANDOM_GET_MAX: u64 = 256;
/// Handles em tabela por processo.
pub const MAX_HANDLES: usize = 4096;
/// Bytes, handles e mensagens em fila de um canal.
pub const CHANNEL_MAX_BYTES: usize = 1024;
pub const CHANNEL_MAX_HANDLES: usize = 8;
pub const CHANNEL_QUEUE_LEN: usize = 16;
/// Pontas de canal que um processo pode ter cobradas a ele (`channel_create` passa disto: `NoMemory`).
pub const MAX_CHANNEL_ENDS: usize = 256;
/// Handles que o kernel aceita copiar de um array de usuário; acima disso é `InvalidArgs`.
pub const USER_HANDLE_ARRAY_MAX: usize = 64;
/// Bytes de heap do kernel que um processo pode ter cobrados a ele (pontas, mensagens em fila,
/// VMOs, processos e threads que criou); acima disso a criação é `NoMemory` (SDD 17, DEBT-0110).
pub const PROCESS_BYTES_MAX: usize = 512 << 10;
/// Bytes do nome que `process_create` aceita.
pub const PROCESS_NAME_MAX: usize = 32;
/// Maior VMO que `vmo_create` aceita; acima disso é `InvalidArgs`.
pub const VMO_MAX_BYTES: u64 = 256 << 20;
/// Páginas da pilha inicial de um programa: 64 KiB, de tamanho fixo (SDD 17, "Layout").
pub const USER_STACK_PAGES: u64 = 16;
/// Onde cai o `mmap_base` de um processo: sorteado entre `2^28` páginas a partir de 64 GiB, e os
/// mapeamentos `ANY` ocupam `[mmap_base, MMAP_TOP)`, abaixo da janela da pilha do kernel.
pub const MMAP_FLOOR: u64 = 0x10_0000_0000;
/// Fim exclusivo do intervalo em que `mmap_base` é sorteado (1 TiB acima de `MMAP_FLOOR`).
pub const MMAP_ASLR_LIMIT: u64 = 0x110_0000_0000;
/// Fim exclusivo da região dos mapeamentos `ANY`.
pub const MMAP_TOP: u64 = 0x6000_0000_0000;
/// O `code` do evento de um processo que `process_kill` encerrou.
pub const PROCESS_KILLED_CODE: i64 = -1024;

/// As chamadas da ABI nativa, com o número que cada uma ocupa em `rax` (x86_64) ou `x8` (aarch64).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Syscall {
    Nop = 0x00,
    ProcessExit = 0x01,
    ThreadExit = 0x02,
    DebugWrite = 0x03,
    ClockGetMonotonic = 0x04,
    RandomGet = 0x05,
    HandleClose = 0x10,
    HandleDuplicate = 0x11,
    HandleReplace = 0x12,
    ObjectWaitOne = 0x13,
    VmoCreate = 0x20,
    VmarMap = 0x21,
    VmarUnmap = 0x22,
    ProcessCreate = 0x30,
    ThreadCreate = 0x31,
    ProcessKill = 0x32,
    ChannelCreate = 0x40,
    ChannelWrite = 0x41,
    ChannelRead = 0x42,
}

impl Syscall {
    /// Todas as chamadas, na ordem do número.
    pub const ALL: [Self; 19] = [
        Self::Nop,
        Self::ProcessExit,
        Self::ThreadExit,
        Self::DebugWrite,
        Self::ClockGetMonotonic,
        Self::RandomGet,
        Self::HandleClose,
        Self::HandleDuplicate,
        Self::HandleReplace,
        Self::ObjectWaitOne,
        Self::VmoCreate,
        Self::VmarMap,
        Self::VmarUnmap,
        Self::ProcessCreate,
        Self::ThreadCreate,
        Self::ProcessKill,
        Self::ChannelCreate,
        Self::ChannelWrite,
        Self::ChannelRead,
    ];

    /// `None` para número que a ABI não define.
    pub const fn from_raw(number: u64) -> Option<Self> {
        let mut index = 0;
        while index < Self::ALL.len() {
            if Self::ALL[index] as u64 == number {
                return Some(Self::ALL[index]);
            }
            index += 1;
        }
        None
    }

    pub const fn raw(self) -> u64 {
        self as u64
    }
}

/// O resultado de uma chamada: `Ok` é 0 e todo erro é negativo (`rax`/`x0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i64)]
pub enum Status {
    Ok = 0,
    InvalidArgs = -1,
    BadHandle = -2,
    WrongType = -3,
    AccessDenied = -4,
    NoMemory = -5,
    BadState = -6,
    NotSupported = -7,
    OutOfRange = -8,
    PeerClosed = -9,
    ShouldWait = -10,
    TimedOut = -11,
    BufferTooSmall = -12,
}

impl Status {
    pub const fn raw(self) -> i64 {
        self as i64
    }

    /// `None` para valor que a ABI não define.
    pub const fn from_raw(raw: i64) -> Option<Self> {
        Some(match raw {
            0 => Self::Ok,
            -1 => Self::InvalidArgs,
            -2 => Self::BadHandle,
            -3 => Self::WrongType,
            -4 => Self::AccessDenied,
            -5 => Self::NoMemory,
            -6 => Self::BadState,
            -7 => Self::NotSupported,
            -8 => Self::OutOfRange,
            -9 => Self::PeerClosed,
            -10 => Self::ShouldWait,
            -11 => Self::TimedOut,
            -12 => Self::BufferTooSmall,
            _ => return None,
        })
    }
}

/// O que o `arch/` entrega ao núcleo numa chamada: o número e os seis argumentos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyscallArgs {
    pub number: u64,
    pub args: [u64; 6],
}

/// O que o núcleo devolve: o status em `rax`/`x0` e o valor em `rdx`/`x1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyscallReturn {
    pub status: i64,
    pub value: u64,
}

impl SyscallReturn {
    pub const fn ok(value: u64) -> Self {
        Self {
            status: Status::Ok.raw(),
            value,
        }
    }

    pub const fn err(status: Status) -> Self {
        Self {
            status: status.raw(),
            value: 0,
        }
    }
}

/// Os direitos de um handle (`u32`): o bit `n` da tabela do SDD 17 vale `1 << n`. Só diminuem.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Rights(u32);

impl Rights {
    pub const NONE: Self = Self(0);
    pub const DUPLICATE: Self = Self(1 << 0);
    pub const TRANSFER: Self = Self(1 << 1);
    pub const READ: Self = Self(1 << 2);
    pub const WRITE: Self = Self(1 << 3);
    pub const EXECUTE: Self = Self(1 << 4);
    pub const MAP: Self = Self(1 << 5);
    pub const WAIT: Self = Self(1 << 6);
    pub const INSPECT: Self = Self(1 << 7);
    pub const DESTROY: Self = Self(1 << 8);
    pub const MANAGE_PROCESS: Self = Self(1 << 9);
    pub const MANAGE_THREAD: Self = Self(1 << 10);
    /// Os bits definidos; de 11 a 31 são reservados.
    pub const ALL: Self = Self((1 << 11) - 1);

    /// `None` se algum bit reservado está ligado (a chamada responde `InvalidArgs`).
    pub const fn from_bits(bits: u32) -> Option<Self> {
        if bits & !Self::ALL.0 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// `self` não tem direito que `other` não tenha.
    pub const fn is_subset_of(self, other: Self) -> bool {
        other.contains(self)
    }

    pub const PROCESS: Self = Self::DUPLICATE
        .union(Self::TRANSFER)
        .union(Self::WAIT)
        .union(Self::INSPECT)
        .union(Self::DESTROY)
        .union(Self::MANAGE_PROCESS)
        .union(Self::MANAGE_THREAD);
    pub const THREAD: Self = Self::DUPLICATE
        .union(Self::TRANSFER)
        .union(Self::WAIT)
        .union(Self::INSPECT);
    pub const VMO: Self = Self::DUPLICATE
        .union(Self::TRANSFER)
        .union(Self::READ)
        .union(Self::WRITE)
        .union(Self::EXECUTE)
        .union(Self::MAP)
        .union(Self::INSPECT);
    pub const VMAR: Self = Self::DUPLICATE
        .union(Self::TRANSFER)
        .union(Self::READ)
        .union(Self::WRITE)
        .union(Self::EXECUTE)
        .union(Self::MAP);
    pub const CHANNEL: Self = Self::DUPLICATE
        .union(Self::TRANSFER)
        .union(Self::READ)
        .union(Self::WRITE)
        .union(Self::WAIT);
    pub const CONSOLE: Self = Self::DUPLICATE.union(Self::TRANSFER).union(Self::WRITE);
    /// Initramfs entregue ao `init`: sem `WRITE` nem `EXECUTE`.
    pub const INITRAMFS: Self = Self::DUPLICATE
        .union(Self::TRANSFER)
        .union(Self::READ)
        .union(Self::MAP);
}

/// Prazos de `object_wait_one` acima de `agora + WAIT_HORIZON_NS` (cerca de 73 anos) valem "sem prazo",
/// como `u64::MAX`: o timer do hardware não recebe valor que dê a volta no contador.
pub const WAIT_HORIZON_NS: u64 = 1 << 61;

/// O prazo que o kernel arma para `deadline_ns` pedido às `now_ns`: `None` é sem prazo.
pub const fn wait_deadline(now_ns: u64, deadline_ns: u64) -> Option<u64> {
    if deadline_ns == u64::MAX || deadline_ns > now_ns.saturating_add(WAIT_HORIZON_NS) {
        None
    } else {
        Some(deadline_ns)
    }
}

/// Sinais de `object_wait_one` (nível, não borda).
pub mod signals {
    pub const READABLE: u32 = 1 << 0;
    pub const WRITABLE: u32 = 1 << 1;
    pub const PEER_CLOSED: u32 = 1 << 2;
    pub const TERMINATED: u32 = 1 << 3;
}

/// Flags de `vmar_map`. A numeração difere dos bits de direito de propósito: flag é argumento
/// de uma chamada, direito é atributo de um handle.
pub mod vmar_flags {
    use super::Rights;

    pub const READ: u32 = 1 << 0;
    pub const WRITE: u32 = 1 << 1;
    pub const EXECUTE: u32 = 1 << 2;
    pub const FIXED: u32 = 1 << 3;
    const KNOWN: u32 = READ | WRITE | EXECUTE | FIXED;

    /// `None` se `flags` tem bit reservado ou pede escrita e execução juntas (W^X).
    pub const fn validate(flags: u32) -> Option<u32> {
        if flags & !KNOWN != 0 || flags & WRITE != 0 && flags & EXECUTE != 0 {
            None
        } else {
            Some(flags)
        }
    }

    /// Os direitos que o mapeamento pede ao VMO e ao VMAR: a tabela explícita flag para direito.
    pub const fn required_rights(flags: u32) -> Rights {
        let mut rights = Rights::MAP;
        if flags & READ != 0 {
            rights = rights.union(Rights::READ);
        }
        if flags & WRITE != 0 {
            rights = rights.union(Rights::WRITE);
        }
        if flags & EXECUTE != 0 {
            rights = rights.union(Rights::EXECUTE);
        }
        rights
    }
}

/// Um handle: `u64` = `geração << 16 | (índice + 1)`; `0` nunca é válido.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Handle(u64);

impl Handle {
    pub const INVALID: Self = Self(0);
    /// Última geração de um slot (48 bits); o slot que chegaria a ela é aposentado.
    pub const MAX_GENERATION: u64 = (1 << 48) - 1;
    /// Slots de uma tabela: o índice mais um cabe em 16 bits.
    pub const MAX_SLOTS: usize = 0xFFFF - 1;

    /// `None` com geração acima de 48 bits ou índice acima de `MAX_SLOTS`.
    pub const fn new(generation: u64, index: usize) -> Option<Self> {
        if generation > Self::MAX_GENERATION || index >= Self::MAX_SLOTS {
            return None;
        }
        Some(Self(generation << 16 | (index as u64 + 1)))
    }

    /// Do valor cru, que o usuário passou: `None` para `0`.
    pub const fn from_raw(raw: u64) -> Option<Self> {
        let slot = raw & 0xFFFF;
        if slot == 0 || slot > Self::MAX_SLOTS as u64 {
            None
        } else {
            Some(Self(raw))
        }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    pub const fn index(self) -> usize {
        (self.0 & 0xFFFF) as usize - 1
    }

    pub const fn generation(self) -> u64 {
        self.0 >> 16
    }
}

/// Os tipos de evento de um processo.
pub mod event_kind {
    pub const EXIT: u32 = 1;
    pub const FAULT: u32 = 2;
}

/// Por que um processo foi morto pela CPU.
pub mod fault_kind {
    pub const NONE: u32 = 0;
    pub const PAGE_FAULT: u32 = 1;
    pub const ILLEGAL_INSTRUCTION: u32 = 2;
    pub const UNALIGNED: u32 = 3;
    pub const ARITHMETIC: u32 = 4;
    pub const BREAKPOINT: u32 = 5;
    pub const OTHER: u32 = 6;
}

/// Bits de `ProcessEvent::access` numa falta de página.
pub mod fault_access {
    pub const WRITE: u32 = 1 << 0;
    pub const EXECUTE: u32 = 1 << 1;
    pub const PROTECTION: u32 = 1 << 2;
}

/// Uma mensagem do canal de eventos do processo: 56 bytes, sem handles. Só endereço de usuário.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct ProcessEvent {
    pub kind: u32,
    pub fault_kind: u32,
    pub access: u32,
    pub reserved: u32,
    pub code: i64,
    pub pc: u64,
    pub sp: u64,
    pub addr: u64,
    pub isa_code: u64,
}

/// Cabeçalho da mensagem de boot: seguido de `count` valores `u32` de `BootKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct BootMessageHeader {
    pub magic: u32,
    pub version: u32,
    pub count: u32,
    pub reserved: u32,
}

impl BootMessageHeader {
    pub const MAGIC: u32 = 0x544F_4F42;
    pub const VERSION: u32 = 1;

    pub const fn new(count: u32) -> Self {
        Self {
            magic: Self::MAGIC,
            version: Self::VERSION,
            count,
            reserved: 0,
        }
    }
}

/// Onde o `channel_read` grava os tamanhos da mensagem (`actual*`): dois `u64` seguidos, como o
/// `out` de `channel_create`. Em `BufferTooSmall` leva o que a mensagem pede.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct ReadActual {
    pub bytes: u64,
    pub handles: u64,
}

/// Escreve a mensagem de boot em `out`: o cabeçalho e um `u32` de `boot_kind` por item, o i-ésimo
/// valor para o i-ésimo handle da mensagem. Devolve o tamanho, ou `None` se não cabe em `out`
/// ou passa de `CHANNEL_MAX_HANDLES` itens.
pub fn encode_boot_message(kinds: &[u32], out: &mut [u8]) -> Option<usize> {
    let len = size_of::<BootMessageHeader>() + kinds.len() * 4;
    if kinds.len() > CHANNEL_MAX_HANDLES || out.len() < len {
        return None;
    }
    let header = BootMessageHeader::new(u32::try_from(kinds.len()).ok()?);
    let words = [header.magic, header.version, header.count, header.reserved];
    for (chunk, word) in out
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(words.iter().chain(kinds))
    {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    Some(len)
}

/// Lê a mensagem de boot: confere magic, versão, `reserved` zero, tamanho exato e o teto de
/// itens, e copia os `boot_kind` para `kinds`. Devolve quantos são, ou `None` se a mensagem não é válida.
pub fn parse_boot_message(bytes: &[u8], kinds: &mut [u32; CHANNEL_MAX_HANDLES]) -> Option<usize> {
    let word = |at: usize| -> Option<u32> {
        let raw: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        Some(u32::from_le_bytes(raw))
    };
    let count = usize::try_from(word(8)?).ok()?;
    let valid = word(0)? == BootMessageHeader::MAGIC
        && word(4)? == BootMessageHeader::VERSION
        && word(12)? == 0
        && count <= CHANNEL_MAX_HANDLES
        && bytes.len() == size_of::<BootMessageHeader>() + count * 4;
    if !valid {
        return None;
    }
    for (index, kind) in kinds.iter_mut().take(count).enumerate() {
        *kind = word(16 + index * 4)?;
    }
    Some(count)
}

/// O que cada handle da mensagem de boot é, na ordem do array de handles.
pub mod boot_kind {
    pub const PROCESS: u32 = 1;
    pub const VMAR_ROOT: u32 = 2;
    pub const CONSOLE: u32 = 3;
    pub const INITRAMFS: u32 = 4;
}

/// O byte passa pelo filtro do `debug_write`: só ASCII imprimível, `\n` e `\t`. ESC, os outros
/// controles, `0x7F` e tudo acima de `0x7F` (o CSI de 8 bits, U+2028 e o bidi vêm em UTF-8) saem.
pub const fn debug_byte_allowed(byte: u8) -> bool {
    byte == b'\n' || byte == b'\t' || byte >= 0x20 && byte < 0x7F
}

/// O texto do `debug_write` como o console o recebe: o byte que não passa vira `?`, cada linha
/// ganha `prefix` na frente, e a saída termina em `\n` (o usuário não emenda a linha do kernel).
pub fn debug_sanitize(bytes: &[u8], prefix: &str, mut emit: impl FnMut(char)) {
    let mut at_line_start = true;
    for &byte in bytes {
        if at_line_start {
            prefix.chars().for_each(&mut emit);
        }
        let ch = if debug_byte_allowed(byte) {
            char::from(byte)
        } else {
            '?'
        };
        emit(ch);
        at_line_start = ch == '\n';
    }
    if !at_line_start {
        emit('\n');
    }
}

/// A pilha inicial de um programa: `argc`, `argv`, `envp` e o auxv (SDD 17, "Auxv mínimo").
pub mod auxv {
    pub const AT_NULL: u64 = 0;
    pub const AT_PHDR: u64 = 3;
    pub const AT_PHENT: u64 = 4;
    pub const AT_PHNUM: u64 = 5;
    pub const AT_PAGESZ: u64 = 6;
    pub const AT_ENTRY: u64 = 9;
    pub const AT_HWCAP: u64 = 16;
    pub const AT_RANDOM: u64 = 25;
    pub const AT_HWCAP2: u64 = 26;
}

/// Bytes de `AT_RANDOM`.
pub const AT_RANDOM_LEN: usize = 16;
/// Tamanho do maior bloco que `InitialStack` monta.
pub const INITIAL_STACK_MAX: usize = 256;
/// `argc`, o fim de `argv` e o de `envp`, mais as 9 entradas do auxv (duas palavras cada).
const MAX_WORDS: usize = 3 + 9 * 2;

/// O que o kernel sabe do programa que vai começar, para o auxv.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct StartInfo {
    pub entry: u64,
    /// Endereço dos program headers na memória, se a tabela cai dentro de um `PT_LOAD`.
    pub phdr: Option<u64>,
    pub phent: u64,
    pub phnum: u64,
    pub random: [u8; AT_RANDOM_LEN],
}

/// Os bytes que o kernel grava no topo da pilha do primeiro programa. `bytes()` ocupa
/// `[sp, top)`: `argc = 0`, `argv` e `envp` vazios, o auxv e, no fim, os bytes de `AT_RANDOM`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct InitialStack {
    sp: u64,
    len: usize,
    bytes: [u8; INITIAL_STACK_MAX],
}

/// Sem os bytes de `AT_RANDOM` (guidelines 09).
impl core::fmt::Debug for StartInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StartInfo")
            .field("entry", &self.entry)
            .field("phdr", &self.phdr)
            .field("phent", &self.phent)
            .field("phnum", &self.phnum)
            .field("random", &"<redacted>")
            .finish()
    }
}

/// Sem os bytes da pilha, que levam os de `AT_RANDOM` (guidelines 09).
impl core::fmt::Debug for InitialStack {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InitialStack")
            .field("sp", &self.sp)
            .field("len", &self.len)
            .field("bytes", &"<redacted>")
            .finish()
    }
}

impl InitialStack {
    /// `top` é o fim exclusivo da pilha, alinhado a 16. `None` se não é.
    pub fn build(top: u64, info: &StartInfo) -> Option<Self> {
        if !top.is_multiple_of(16) || top < USER_MIN + INITIAL_STACK_MAX as u64 {
            return None;
        }
        let random_at = top - AT_RANDOM_LEN as u64;
        let mut words = [0_u64; MAX_WORDS];
        let mut count = 3;
        let mut push = |tag: u64, value: u64| {
            words[count] = tag;
            words[count + 1] = value;
            count += 2;
        };
        if let Some(phdr) = info.phdr {
            push(auxv::AT_PHDR, phdr);
            push(auxv::AT_PHENT, info.phent);
            push(auxv::AT_PHNUM, info.phnum);
        }
        push(auxv::AT_PAGESZ, PAGE_SIZE);
        push(auxv::AT_ENTRY, info.entry);
        push(auxv::AT_HWCAP, 0);
        push(auxv::AT_HWCAP2, 0);
        push(auxv::AT_RANDOM, random_at);
        push(auxv::AT_NULL, 0);
        let head = count * 8;
        let len = (head + AT_RANDOM_LEN).next_multiple_of(16);
        let mut bytes = [0_u8; INITIAL_STACK_MAX];
        for (slot, word) in bytes.as_chunks_mut::<8>().0.iter_mut().zip(&words[..count]) {
            slot.copy_from_slice(&word.to_le_bytes());
        }
        bytes[len - AT_RANDOM_LEN..len].copy_from_slice(&info.random);
        Some(Self {
            sp: top - len as u64,
            len,
            bytes,
        })
    }

    /// O valor inicial do ponteiro de pilha: aponta `argc`, alinhado a 16.
    pub const fn sp(&self) -> u64 {
        self.sp
    }

    /// Os bytes de `[sp, top)`.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// Todo o armazenamento, para o chamador apagá-lo depois de copiar `bytes()` para o frame.
    pub fn storage_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}

#[cfg(test)]
mod tests {
    use core::mem::{offset_of, size_of};

    use super::*;

    #[test]
    fn every_syscall_has_the_number_of_the_table() {
        let expected = [
            (Syscall::Nop, 0x00),
            (Syscall::ProcessExit, 0x01),
            (Syscall::ThreadExit, 0x02),
            (Syscall::DebugWrite, 0x03),
            (Syscall::ClockGetMonotonic, 0x04),
            (Syscall::RandomGet, 0x05),
            (Syscall::HandleClose, 0x10),
            (Syscall::HandleDuplicate, 0x11),
            (Syscall::HandleReplace, 0x12),
            (Syscall::ObjectWaitOne, 0x13),
            (Syscall::VmoCreate, 0x20),
            (Syscall::VmarMap, 0x21),
            (Syscall::VmarUnmap, 0x22),
            (Syscall::ProcessCreate, 0x30),
            (Syscall::ThreadCreate, 0x31),
            (Syscall::ProcessKill, 0x32),
            (Syscall::ChannelCreate, 0x40),
            (Syscall::ChannelWrite, 0x41),
            (Syscall::ChannelRead, 0x42),
        ];
        assert_eq!(expected.len(), Syscall::ALL.len());
        for (call, number) in expected {
            assert_eq!(call.raw(), number);
            assert_eq!(Syscall::from_raw(number), Some(call));
        }
    }

    #[test]
    fn unknown_numbers_are_not_syscalls() {
        for number in [0x06, 0x0F, 0x14, 0x23, 0x33, 0x43, 0x100, u64::MAX] {
            assert_eq!(Syscall::from_raw(number), None);
        }
    }

    #[test]
    fn status_values_match_the_table_and_round_trip() {
        let expected = [
            (Status::Ok, 0),
            (Status::InvalidArgs, -1),
            (Status::BadHandle, -2),
            (Status::WrongType, -3),
            (Status::AccessDenied, -4),
            (Status::NoMemory, -5),
            (Status::BadState, -6),
            (Status::NotSupported, -7),
            (Status::OutOfRange, -8),
            (Status::PeerClosed, -9),
            (Status::ShouldWait, -10),
            (Status::TimedOut, -11),
            (Status::BufferTooSmall, -12),
        ];
        for (status, raw) in expected {
            assert_eq!(status.raw(), raw);
            assert_eq!(Status::from_raw(raw), Some(status));
        }
        assert_eq!(Status::from_raw(1), None);
        assert_eq!(Status::from_raw(-13), None);
    }

    #[test]
    fn rights_bits_follow_the_table_and_reserved_bits_are_refused() {
        assert_eq!(Rights::DUPLICATE.bits(), 1);
        assert_eq!(Rights::READ.bits(), 1 << 2);
        assert_eq!(Rights::MANAGE_THREAD.bits(), 1 << 10);
        assert_eq!(Rights::ALL.bits(), 0x7FF);
        assert_eq!(Rights::from_bits(0x7FF), Some(Rights::ALL));
        assert_eq!(Rights::from_bits(1 << 11), None);
        assert_eq!(Rights::from_bits(u32::MAX), None);
    }

    #[test]
    fn rights_only_shrink_by_subset() {
        assert!(Rights::READ.is_subset_of(Rights::VMO));
        assert!(!Rights::VMO.is_subset_of(Rights::READ));
        assert!(Rights::NONE.is_subset_of(Rights::NONE));
        assert!(!Rights::DESTROY.is_subset_of(Rights::CHANNEL));
    }

    #[test]
    fn initial_rights_of_each_object_match_the_table() {
        assert!(!Rights::INITRAMFS.contains(Rights::EXECUTE));
        assert!(!Rights::INITRAMFS.contains(Rights::WRITE));
        assert!(Rights::INITRAMFS.contains(Rights::MAP));
        assert!(!Rights::CONSOLE.contains(Rights::READ));
        assert!(Rights::PROCESS.contains(Rights::MANAGE_PROCESS));
        assert!(!Rights::THREAD.contains(Rights::DESTROY));
        assert!(Rights::CHANNEL.contains(Rights::WAIT));
        assert!(!Rights::VMAR.contains(Rights::WAIT));
    }

    #[test]
    fn vmar_flags_and_rights_use_different_numbers_with_an_explicit_conversion() {
        assert_ne!(vmar_flags::READ, Rights::READ.bits());
        assert_ne!(vmar_flags::WRITE, Rights::WRITE.bits());
        assert_ne!(vmar_flags::EXECUTE, Rights::EXECUTE.bits());
        let rights = vmar_flags::required_rights(vmar_flags::READ | vmar_flags::WRITE);
        assert_eq!(rights, Rights::MAP.union(Rights::READ).union(Rights::WRITE));
        assert!(!rights.contains(Rights::EXECUTE));
    }

    #[test]
    fn vmar_flags_refuse_reserved_bits_and_write_plus_execute() {
        assert_eq!(vmar_flags::validate(vmar_flags::READ), Some(1));
        assert_eq!(vmar_flags::validate(1 << 4), None);
        assert_eq!(
            vmar_flags::validate(vmar_flags::WRITE | vmar_flags::EXECUTE),
            None
        );
        assert!(vmar_flags::validate(vmar_flags::EXECUTE | vmar_flags::FIXED).is_some());
    }

    #[test]
    fn handle_zero_is_never_valid_and_the_first_one_is_one() {
        assert_eq!(Handle::from_raw(0), None);
        assert_eq!(Handle::from_raw(1 << 16), None);
        assert_eq!(Handle::new(0, 0).map(Handle::raw), Some(1));
    }

    #[test]
    fn handle_packs_generation_and_index_plus_one() {
        let handle = Handle::new(0xABCD_EF01_2345, 7).unwrap();
        assert_eq!(handle.raw(), 0xABCD_EF01_2345 << 16 | 8);
        assert_eq!(handle.index(), 7);
        assert_eq!(handle.generation(), 0xABCD_EF01_2345);
        assert_eq!(Handle::from_raw(handle.raw()), Some(handle));
    }

    #[test]
    fn handle_limits_are_the_48_bit_generation_and_the_16_bit_index() {
        assert!(Handle::new(Handle::MAX_GENERATION, 0).is_some());
        assert!(Handle::new(Handle::MAX_GENERATION + 1, 0).is_none());
        assert!(Handle::new(0, Handle::MAX_SLOTS - 1).is_some());
        assert!(Handle::new(0, Handle::MAX_SLOTS).is_none());
        let last = Handle::new(0, Handle::MAX_SLOTS - 1).unwrap();
        assert_eq!(last.raw() & 0xFFFF, 0xFFFE);
        assert_eq!(Handle::from_raw(last.raw()), Some(last));
    }

    #[test]
    fn from_raw_accepts_exactly_what_new_can_build() {
        assert_eq!(Handle::from_raw(0xFFFF), None);
        assert_eq!(Handle::from_raw(7 << 16 | 0xFFFF), None);
        assert!(Handle::from_raw(0xFFFE).is_some());
        for index in [0, 1, 4095, Handle::MAX_SLOTS - 1] {
            let built = Handle::new(5, index).unwrap();
            assert_eq!(Handle::from_raw(built.raw()), Some(built));
        }
        for slot in 0..=0xFFFF_u64 {
            let accepted = Handle::from_raw(slot).is_some();
            assert_eq!(
                accepted,
                (1..=Handle::MAX_SLOTS as u64).contains(&slot),
                "{slot}"
            );
        }
    }

    #[test]
    fn process_event_layout_is_56_bytes_at_the_documented_offsets() {
        assert_eq!(size_of::<ProcessEvent>(), 56);
        assert_eq!(offset_of!(ProcessEvent, kind), 0);
        assert_eq!(offset_of!(ProcessEvent, fault_kind), 4);
        assert_eq!(offset_of!(ProcessEvent, access), 8);
        assert_eq!(offset_of!(ProcessEvent, reserved), 12);
        assert_eq!(offset_of!(ProcessEvent, code), 16);
        assert_eq!(offset_of!(ProcessEvent, pc), 24);
        assert_eq!(offset_of!(ProcessEvent, sp), 32);
        assert_eq!(offset_of!(ProcessEvent, addr), 40);
        assert_eq!(offset_of!(ProcessEvent, isa_code), 48);
    }

    #[test]
    fn boot_message_header_is_16_bytes_with_the_magic_of_the_table() {
        assert_eq!(size_of::<BootMessageHeader>(), 16);
        let header = BootMessageHeader::new(4);
        assert_eq!(
            (header.magic, header.version, header.count),
            (0x544F_4F42, 1, 4)
        );
        assert_eq!(BootMessageHeader::MAGIC.to_le_bytes(), *b"BOOT");
    }

    const _: () = assert!(16 + CHANNEL_MAX_HANDLES * 4 <= CHANNEL_MAX_BYTES);
    const _: () = assert!(USER_HANDLE_ARRAY_MAX >= CHANNEL_MAX_HANDLES);

    #[test]
    fn the_mmap_window_sits_between_the_image_and_the_stack_window() {
        let page = PAGE_SIZE;
        assert!(MMAP_FLOOR > USER_IMAGE_BASE && MMAP_FLOOR.is_multiple_of(page));
        assert_eq!(MMAP_ASLR_LIMIT - MMAP_FLOOR, (1 << 28) * page);
        assert!(MMAP_ASLR_LIMIT < MMAP_TOP && MMAP_TOP <= 0x6000_0000_0000);
        assert!(MMAP_TOP < USER_TOP);
    }

    #[test]
    fn the_byte_cap_and_the_limits_that_it_bounds_are_consistent() {
        assert!(PROCESS_BYTES_MAX >= 64 * 1024);
        assert!(PROCESS_BYTES_MAX < MAX_CHANNEL_ENDS * 4096);
        assert!(VMO_MAX_BYTES.is_multiple_of(PAGE_SIZE));
        assert!(PROCESS_KILLED_CODE < 0);
    }

    #[test]
    fn a_wait_deadline_beyond_the_horizon_means_no_deadline() {
        let now = 5_000_000_000;
        assert_eq!(wait_deadline(now, 0), Some(0));
        assert_eq!(wait_deadline(now, now + 1), Some(now + 1));
        assert_eq!(
            wait_deadline(now, now + WAIT_HORIZON_NS),
            Some(now + WAIT_HORIZON_NS)
        );
        assert_eq!(wait_deadline(now, now + WAIT_HORIZON_NS + 1), None);
        assert_eq!(wait_deadline(now, 1 << 63), None);
        assert_eq!(wait_deadline(now, u64::MAX), None);
        assert_eq!(
            wait_deadline(u64::MAX - 5, u64::MAX - 1),
            Some(u64::MAX - 1)
        );
    }

    #[test]
    fn read_actual_is_two_u64_in_a_row() {
        assert_eq!(size_of::<ReadActual>(), 16);
        assert_eq!(offset_of!(ReadActual, bytes), 0);
        assert_eq!(offset_of!(ReadActual, handles), 8);
    }

    #[test]
    fn the_boot_message_round_trips_and_the_kinds_follow_the_header() {
        let mut out = [0_u8; 64];
        let len = encode_boot_message(&[boot_kind::PROCESS, boot_kind::CONSOLE], &mut out).unwrap();
        assert_eq!(len, 24);
        assert_eq!(&out[..4], b"BOOT");
        assert_eq!(&out[16..20], &1_u32.to_le_bytes());
        assert_eq!(&out[20..24], &3_u32.to_le_bytes());
        let mut kinds = [0; CHANNEL_MAX_HANDLES];
        assert_eq!(parse_boot_message(&out[..len], &mut kinds), Some(2));
        assert_eq!(&kinds[..2], &[boot_kind::PROCESS, boot_kind::CONSOLE]);
    }

    #[test]
    fn the_boot_message_refuses_what_is_not_the_contract() {
        let mut out = [0_u8; 64];
        let len = encode_boot_message(&[1, 2, 3, 4], &mut out).unwrap();
        let mut kinds = [0; CHANNEL_MAX_HANDLES];
        assert_eq!(parse_boot_message(&out[..len], &mut kinds), Some(4));
        assert_eq!(parse_boot_message(&out[..len - 1], &mut kinds), None);
        assert_eq!(parse_boot_message(&out[..len + 4], &mut kinds), None);
        assert_eq!(parse_boot_message(&out[..15], &mut kinds), None);
        assert_eq!(parse_boot_message(&[], &mut kinds), None);
        for (at, byte) in [(0, 0x00), (4, 2), (12, 1), (8, 9)] {
            let mut bad = out;
            bad[at] = byte;
            assert_eq!(parse_boot_message(&bad[..len], &mut kinds), None, "{at}");
        }
        assert!(encode_boot_message(&[0; CHANNEL_MAX_HANDLES + 1], &mut [0; 128]).is_none());
        assert!(encode_boot_message(&[1, 2], &mut [0; 23]).is_none());
        let mut full = [0_u8; 16 + 4 * CHANNEL_MAX_HANDLES];
        let eight = [boot_kind::PROCESS; CHANNEL_MAX_HANDLES];
        assert_eq!(encode_boot_message(&eight, &mut full), Some(full.len()));
    }

    const _: () = assert!(USER_MIN < USER_IMAGE_BASE && USER_IMAGE_BASE < USER_TOP);
    const _: () = assert!(USER_TOP < 1 << 47);

    #[test]
    fn user_layout_leaves_the_null_zone_and_the_last_page() {
        assert_eq!(USER_MIN, 0x1_0000);
        assert_eq!(USER_TOP, 0x0000_7FFF_FFFF_F000);
        assert!(USER_TOP.is_multiple_of(PAGE_SIZE));
    }

    #[test]
    fn debug_filter_keeps_only_printable_ascii_newline_and_tab() {
        assert!(debug_byte_allowed(b'a'));
        assert!(debug_byte_allowed(b'\n'));
        assert!(debug_byte_allowed(b'\t'));
        assert!(debug_byte_allowed(b' '));
        assert!(debug_byte_allowed(b'~'));
        for byte in [0x1B, 0x00, 0x07, 0x08, 0x0D, 0x7F, 0x80, 0x9B, 0xC2, 0xFF] {
            assert!(!debug_byte_allowed(byte), "{byte:#x}");
        }
    }

    fn sanitized(bytes: &[u8]) -> String {
        let mut out = String::new();
        debug_sanitize(bytes, "[user] ", |ch| out.push(ch));
        out
    }

    #[test]
    fn sanitize_turns_non_ascii_controls_into_question_marks() {
        for text in ["\u{9b}31m", "a\u{2028}b", "a\u{2029}b", "x\u{202e}y"] {
            let out = sanitized(text.as_bytes());
            assert!(out.is_ascii(), "{out:?}");
            assert!(!out.contains('\u{9b}') && !out.contains('\u{2028}'));
            assert_eq!(
                out.matches('?').count(),
                text.len() - text.chars().filter(char::is_ascii).count()
            );
        }
        assert_eq!(sanitized(b"\x1b[31mred"), "[user] ?[31mred\n");
    }

    #[test]
    fn sanitize_prefixes_every_line_and_always_ends_the_output_with_a_newline() {
        assert_eq!(sanitized(b"a\nb"), "[user] a\n[user] b\n");
        assert_eq!(sanitized(b"a\n"), "[user] a\n");
        assert_eq!(sanitized(b"a\n\nb\n"), "[user] a\n[user] \n[user] b\n");
        assert_eq!(sanitized(b""), "");
    }

    #[test]
    fn a_user_line_cannot_start_with_a_kernel_marker() {
        let out = sanitized(b"[rustos] boot ok\n[rustos] panic: x");
        assert!(out.lines().all(|line| line.starts_with("[user] ")));
    }

    #[test]
    fn syscall_return_helpers_put_the_status_apart_from_the_value() {
        assert_eq!(SyscallReturn::ok(5).status, 0);
        assert_eq!(SyscallReturn::ok(5).value, 5);
        assert_eq!(SyscallReturn::err(Status::NotSupported).status, -7);
        assert_eq!(SyscallReturn::err(Status::NotSupported).value, 0);
    }

    const TOP: u64 = 0x7000_0000_0000;
    const RANDOM: [u8; AT_RANDOM_LEN] = *b"0123456789abcdef";

    fn start(phdr: Option<u64>) -> StartInfo {
        StartInfo {
            entry: 0x40_1000,
            phdr,
            phent: 56,
            phnum: 4,
            random: RANDOM,
        }
    }

    fn word(stack: &InitialStack, index: usize) -> u64 {
        let bytes = &stack.bytes()[index * 8..index * 8 + 8];
        u64::from_le_bytes(bytes.try_into().unwrap())
    }

    /// As entradas do auxv depois de `argc`, `argv` e `envp`, até `AT_NULL` inclusive.
    fn auxv_of(stack: &InitialStack) -> Vec<(u64, u64)> {
        let mut entries = Vec::new();
        let mut index = 3;
        loop {
            let pair = (word(stack, index), word(stack, index + 1));
            entries.push(pair);
            if pair.0 == auxv::AT_NULL {
                return entries;
            }
            index += 2;
        }
    }

    #[test]
    fn the_initial_stack_has_argc_empty_argv_and_envp_then_the_auxv() {
        let stack = InitialStack::build(TOP, &start(Some(0x40_0040))).unwrap();
        assert_eq!(
            (word(&stack, 0), word(&stack, 1), word(&stack, 2)),
            (0, 0, 0)
        );
        let random_at = TOP - AT_RANDOM_LEN as u64;
        assert_eq!(
            auxv_of(&stack),
            [
                (auxv::AT_PHDR, 0x40_0040),
                (auxv::AT_PHENT, 56),
                (auxv::AT_PHNUM, 4),
                (auxv::AT_PAGESZ, 4096),
                (auxv::AT_ENTRY, 0x40_1000),
                (auxv::AT_HWCAP, 0),
                (auxv::AT_HWCAP2, 0),
                (auxv::AT_RANDOM, random_at),
                (auxv::AT_NULL, 0),
            ]
        );
    }

    #[test]
    fn the_initial_stack_is_aligned_and_puts_the_random_bytes_at_the_top() {
        for phdr in [Some(0x40_0040), None] {
            let stack = InitialStack::build(TOP, &start(phdr)).unwrap();
            assert_eq!(stack.sp() % 16, 0);
            assert_eq!(stack.sp() + stack.bytes().len() as u64, TOP);
            assert_eq!(stack.bytes().len() % 16, 0);
            assert!(stack.bytes().len() <= INITIAL_STACK_MAX);
            assert_eq!(
                &stack.bytes()[stack.bytes().len() - AT_RANDOM_LEN..],
                &RANDOM
            );
        }
    }

    #[test]
    fn without_a_phdr_inside_a_load_the_auxv_leaves_the_three_entries_out() {
        let stack = InitialStack::build(TOP, &start(None)).unwrap();
        let tags: Vec<u64> = auxv_of(&stack).iter().map(|pair| pair.0).collect();
        assert!(!tags.contains(&auxv::AT_PHDR) && !tags.contains(&auxv::AT_PHNUM));
        assert!(tags.contains(&auxv::AT_RANDOM) && tags.contains(&auxv::AT_ENTRY));
    }

    #[test]
    fn the_initial_stack_refuses_a_top_that_is_not_aligned_or_too_low() {
        assert!(InitialStack::build(TOP + 8, &start(None)).is_none());
        assert!(InitialStack::build(0, &start(None)).is_none());
        assert!(InitialStack::build(USER_MIN, &start(None)).is_none());
    }

    #[test]
    fn auxv_tags_are_the_linux_numbers() {
        let linux = [
            (auxv::AT_NULL, 0),
            (auxv::AT_PHDR, 3),
            (auxv::AT_PHENT, 4),
            (auxv::AT_PHNUM, 5),
            (auxv::AT_PAGESZ, 6),
            (auxv::AT_ENTRY, 9),
            (auxv::AT_HWCAP, 16),
            (auxv::AT_RANDOM, 25),
            (auxv::AT_HWCAP2, 26),
        ];
        for (tag, number) in linux {
            assert_eq!(tag, number);
        }
    }

    #[test]
    fn the_executable_limit_is_64_mib() {
        assert_eq!(EXEC_MAX_LOADS_BYTES, 64 * 1024 * 1024);
    }

    #[test]
    fn debug_never_prints_the_random_bytes() {
        let info = start(Some(0x40_0040));
        let stack = InitialStack::build(TOP, &info).unwrap();
        for text in [format!("{info:?}"), format!("{stack:?}")] {
            assert!(text.contains("<redacted>"), "{text}");
            assert!(
                !text.contains("0123456789abcdef") && !text.contains("48, 49, 50"),
                "{text}"
            );
        }
    }
}
