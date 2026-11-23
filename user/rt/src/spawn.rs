//! O carregador do lado do usuário (SDD 17, "Como o `init` e os outros programas chegam à memória"):
//! lê um ELF com `core/elf`, cria o processo, preenche um VMO por segmento, mapeia no filho com a
//! permissão do segmento, monta a pilha inicial com `hal::abi::InitialStack` (o mesmo código do
//! carregador do kernel) e escreve a mensagem de boot. O kernel garante W^X, faixas e direitos.

use rustos_elf::{self as elf, Image, Limits, PF_W, PF_X, Segment};
use rustos_hal::PAGE_SIZE;
use rustos_hal::abi::{
    AT_RANDOM_LEN, CHANNEL_MAX_HANDLES, EXEC_MAX_LOADS_BYTES, Handle, InitialStack, Rights,
    StartInfo, Status, USER_MIN, USER_STACK_PAGES, USER_TOP, boot_kind, encode_boot_message,
    vmar_flags,
};

use crate::boot::Boot;
use crate::{sys, vm};

const MACHINE: u16 = if cfg!(target_arch = "x86_64") {
    elf::EM_X86_64
} else {
    elf::EM_AARCH64
};

/// Por que um programa não virou processo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Elf(elf::Error),
    Sys(Status),
    /// A pilha inicial não coube no bloco do `hal`: não acontece com a pilha fixa.
    Stack,
}

impl From<Status> for Error {
    fn from(status: Status) -> Self {
        Self::Sys(status)
    }
}

/// O que o programa que cria precisa de si mesmo: o processo (com `MANAGE_PROCESS`) e o VMAR raiz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parent {
    pub process: Handle,
    pub vmar: Handle,
}

impl Parent {
    /// Os handles da mensagem de boot; `None` se o pai não mandou um deles.
    pub fn from_boot(boot: &Boot) -> Option<Self> {
        Some(Self {
            process: boot.process?,
            vmar: boot.vmar_root?,
        })
    }
}

/// Um processo montado e ainda parado: a ponta do canal de boot que o pai guarda e onde começar.
#[derive(Debug)]
pub struct Child {
    pub process: Handle,
    pub vmar: Handle,
    /// A ponta do pai do canal de boot (a do filho é o handle 1 dele).
    pub channel: Handle,
    image: Image,
    parent: Parent,
}

fn limits() -> Limits {
    Limits {
        machine: MACHINE,
        user_min: USER_MIN,
        user_top: USER_TOP,
        page_size: PAGE_SIZE,
        max_loads_bytes: EXEC_MAX_LOADS_BYTES,
    }
}

/// Cria o processo para `file`: valida o ELF, abre o canal de boot e mapeia cada `PT_LOAD`. Ainda
/// sem pilha nem thread.
pub fn create(parent: Parent, file: &[u8]) -> Result<Child, Error> {
    create_with(parent, file, None)
}

/// Como `create`, com `events` (uma ponta com `WRITE` e `TRANSFER`) como canal de eventos do filho: o
/// kernel a move e escreve nela o evento do fim. Consome `events` em qualquer resultado.
pub fn create_with_events(parent: Parent, file: &[u8], events: Handle) -> Result<Child, Error> {
    create_with(parent, file, Some(events))
}

fn create_with(parent: Parent, file: &[u8], events: Option<Handle>) -> Result<Child, Error> {
    let image = match elf::parse(file, &limits()) {
        Ok(image) => image,
        Err(error) => {
            if let Some(events) = events {
                let _ = sys::handle_close(events);
            }
            return Err(Error::Elf(error));
        }
    };
    let (mine, theirs) = match sys::channel_create() {
        Ok(pair) => pair,
        Err(status) => {
            if let Some(events) = events {
                let _ = sys::handle_close(events);
            }
            return Err(Error::Sys(status));
        }
    };
    let (process, vmar) = match sys::process_create_with_events(
        parent.process,
        b"child",
        theirs,
        events.unwrap_or(Handle::INVALID),
    ) {
        Ok(created) => created,
        Err(status) => {
            let _ = sys::handle_close(mine);
            return Err(Error::Sys(status));
        }
    };
    let child = Child {
        process,
        vmar,
        channel: mine,
        image,
        parent,
    };
    for segment in image.loads() {
        if let Err(error) = load_segment(&child, file, segment) {
            child.abandon();
            return Err(error);
        }
    }
    Ok(child)
}

/// Um segmento: o VMO é preenchido por um mapeamento do próprio pai e mapeado no filho com a
/// permissão do segmento, no endereço em que o linker o pôs.
fn load_segment(child: &Child, file: &[u8], segment: &Segment) -> Result<(), Error> {
    let (start, end) = segment.page_range(PAGE_SIZE);
    let len = vm::to_usize(end - start);
    let bytes = segment
        .file_bytes(file)
        .ok_or(Error::Elf(elf::Error::SegmentOutOfFile))?;
    let (vmo, mut local) = vm::scratch(child.parent.vmar, len)?;
    let lead = vm::to_usize(segment.page_offset(PAGE_SIZE));
    local.bytes_mut()[lead..lead + bytes.len()].copy_from_slice(bytes);
    drop(local);
    let mut flags = vmar_flags::READ | vmar_flags::FIXED;
    if segment.flags & PF_W != 0 {
        flags |= vmar_flags::WRITE;
    }
    if segment.flags & PF_X != 0 {
        flags |= vmar_flags::EXECUTE;
    }
    let mapped = sys::vmar_map(child.vmar, vmo, 0, len as u64, flags, start);
    let _ = sys::handle_close(vmo);
    mapped.map(drop).map_err(Error::Sys)
}

impl Child {
    /// Uma pilha nova no filho: `USER_STACK_PAGES` páginas com uma página sem mapa (reservada, que
    /// sempre falta) de cada lado, e o bloco inicial (`argc`, `argv`, `envp`, auxv) no topo. Devolve
    /// o `sp` de entrada.
    pub fn stack(&self) -> Result<u64, Error> {
        let pages = USER_STACK_PAGES;
        let whole = (pages + 2) * PAGE_SIZE;
        let vmo = sys::vmo_create(whole)?;
        let built = self.map_stack(vmo, pages);
        let _ = sys::handle_close(vmo);
        built
    }

    fn map_stack(&self, vmo: Handle, pages: u64) -> Result<u64, Error> {
        let whole = (pages + 2) * PAGE_SIZE;
        let base = sys::vmar_map(self.vmar, vmo, 0, whole, 0, 0)?;
        sys::vmar_unmap(self.vmar, base + PAGE_SIZE, pages * PAGE_SIZE)?;
        let rw = vmar_flags::READ | vmar_flags::WRITE | vmar_flags::FIXED;
        sys::vmar_map(
            self.vmar,
            vmo,
            PAGE_SIZE,
            pages * PAGE_SIZE,
            rw,
            base + PAGE_SIZE,
        )?;
        let top = base + (pages + 1) * PAGE_SIZE;
        let mut random = [0_u8; AT_RANDOM_LEN];
        sys::random_get(&mut random)?;
        let info = StartInfo {
            entry: self.image.entry,
            phdr: self.image.phdr,
            phent: elf::PHDR_LEN as u64,
            phnum: u64::from(self.image.phnum),
            random,
        };
        let mut initial = InitialStack::build(top, &info, &[]).ok_or(Error::Stack)?;
        let last = pages * PAGE_SIZE;
        let mut page = vm::Mapping::new(
            self.parent.vmar,
            vmo,
            last,
            vm::to_usize(PAGE_SIZE),
            vmar_flags::READ | vmar_flags::WRITE,
        )?;
        let bytes = initial.bytes();
        let at = vm::to_usize(PAGE_SIZE) - bytes.len();
        page.bytes_mut()[at..].copy_from_slice(bytes);
        initial.storage_mut().fill(0);
        Ok(initial.sp())
    }

    /// Escreve a mensagem de boot: o processo e o VMAR raiz do filho, e `extra` (o console, por
    /// exemplo), na ordem. Os handles de `extra` saem da tabela do pai.
    pub fn send_boot(&self, extra: &[(u32, Handle)]) -> Result<(), Error> {
        let own = sys::handle_duplicate(self.process, Rights::PROCESS)?;
        let root = sys::handle_duplicate(self.vmar, Rights::VMAR)?;
        let mut kinds = [0_u32; CHANNEL_MAX_HANDLES];
        let mut handles = [Handle::INVALID; CHANNEL_MAX_HANDLES];
        kinds[0] = boot_kind::PROCESS;
        handles[0] = own;
        kinds[1] = boot_kind::VMAR_ROOT;
        handles[1] = root;
        let mut count = 2;
        for &(kind, handle) in extra.iter().take(CHANNEL_MAX_HANDLES - 2) {
            kinds[count] = kind;
            handles[count] = handle;
            count += 1;
        }
        let mut message = [0_u8; 16 + 4 * CHANNEL_MAX_HANDLES];
        let len = encode_boot_message(&kinds[..count], &mut message).ok_or(Error::Stack)?;
        sys::channel_write(self.channel, &message[..len], &handles[..count])?;
        Ok(())
    }

    /// O endereço da entrada do programa.
    pub const fn entry(&self) -> u64 {
        self.image.entry
    }

    /// Inicia o processo: a primeira thread, com `sp` e o handle de boot (`1`) em `rdi`/`x0`.
    pub fn start(&self, sp: u64) -> Result<Handle, Error> {
        Ok(sys::thread_create(
            self.process,
            self.image.entry,
            sp,
            1,
            0,
        )?)
    }

    /// Uma thread a mais, com a própria pilha (a mesma entrada do programa).
    pub fn spawn_thread(&self) -> Result<Handle, Error> {
        let sp = self.stack()?;
        self.start(sp)
    }

    /// Espera o filho terminar até `deadline_ns` (relógio monotônico; `u64::MAX`: sem fim).
    pub fn wait(&self, deadline_ns: u64) -> Result<(), Status> {
        let seen = sys::object_wait_one(self.process, crate::signals::TERMINATED, deadline_ns)?;
        if seen & crate::signals::TERMINATED != 0 {
            Ok(())
        } else {
            Err(Status::TimedOut)
        }
    }

    /// Mata o processo e solta os handles que o pai guarda dele.
    pub fn abandon(&self) {
        let _ = sys::process_kill(self.process);
        let _ = sys::handle_close(self.channel);
        let _ = sys::handle_close(self.process);
        let _ = sys::handle_close(self.vmar);
    }

    /// Solta os handles do pai sem matar o processo: o filho segue sozinho.
    pub fn release(&self) {
        let _ = sys::handle_close(self.channel);
        let _ = sys::handle_close(self.vmar);
        let _ = sys::handle_close(self.process);
    }
}
