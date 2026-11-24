//! O que o processo recebe ao nascer (SDD 17, "O canal de boot"): o programa do initramfs ganha a
//! ponta de um canal com a mensagem de boot já escrita, na ordem dos handles que ela descreve.

use alloc::sync::Arc;

use rustos_hal::PAGE_SIZE;
use rustos_hal::abi::{
    CHANNEL_MAX_BYTES, CHANNEL_MAX_HANDLES, FramebufferFormat, Handle, Rights, Status, boot_kind,
    encode_boot_message,
};
use rustos_object::{HandleTable, Lock, channel_read, channel_write, close};

use super::{Options, Process, VmarObj, system};
use crate::framebuffer;
use crate::object::{Bytes, ConsoleObj, Obj, Table, VecSlots, Work, new_channel};
use crate::sync::IrqLock;
use crate::vmo::Vmo;

/// Os itens da mensagem de boot, na ordem dos handles: o processo, o VMAR raiz e o console; o VMO do
/// initramfs, só para o `init`, e o do framebuffer, só para o shell, vêm depois.
const ITEMS_MAX: usize = 5;

/// A ponta do canal de boot que o kernel guarda (só o modo de teste): fala com o programa por uma
/// tabela de handles própria, como o pai de um processo falaria.
#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "só o modo de teste guarda a ponta")
)]
#[derive(Debug)]
pub struct BootPeer {
    table: IrqLock<Table>,
    channel: Handle,
}

#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "só o modo de teste guarda a ponta")
)]
impl BootPeer {
    /// Escreve `bytes` na fila do programa.
    pub fn write(&self, bytes: &[u8]) -> Result<(), Status> {
        channel_write(
            &self.table,
            self.channel,
            Bytes::uncharged(bytes).ok_or(Status::NoMemory)?,
            &[],
            &mut Work::new(),
        )
    }

    /// Lê a mensagem da frente (sem handles), se houver.
    pub fn read(&self) -> Result<alloc::vec::Vec<u8>, Status> {
        let mut got = alloc::vec::Vec::new();
        channel_read(
            &self.table,
            self.channel,
            CHANNEL_MAX_BYTES,
            CHANNEL_MAX_HANDLES,
            |bytes, _| {
                got.extend_from_slice(bytes);
                true
            },
        )
        .map_err(|failed| failed.status)?;
        Ok(got)
    }

    /// Fecha a ponta: o programa vê `PEER_CLOSED`.
    pub fn close(self) {
        let mut work = Work::new();
        if let Ok(transit) = self.table.with(|table| table.remove(self.channel)) {
            close(transit, &mut work);
        }
    }
}

/// Cria o canal de boot do processo e devolve o valor de `rdi`/`x0` (a ponta dele) e, se `keep`, a
/// ponta do kernel.
pub fn handles_of(
    process: &Arc<Process>,
    options: Options,
) -> Result<(u64, Option<BootPeer>), Status> {
    boot_channel(process, options)
}

/// Cria o par, escreve a mensagem de boot por uma tabela do próprio kernel (a mesma
/// `channel_write` que o usuário usa) e entrega a outra ponta à tabela do processo.
fn boot_channel(
    process: &Arc<Process>,
    options: Options,
) -> Result<(u64, Option<BootPeer>), Status> {
    let (sender, receiver) = new_channel([None, None]);
    let kernel = IrqLock::new(HandleTable::new(VecSlots::default()));
    let put = |object: Obj, rights: Rights| {
        kernel
            .with(|table| table.insert(object, rights))
            .map_err(|_| Status::NoMemory)
    };
    let channel = put(sender, Rights::CHANNEL)?;
    let mut kinds = [0_u32; ITEMS_MAX];
    let mut items = [Handle::INVALID; ITEMS_MAX];
    let mut count = 0;
    let mut add = |kind: u32, handle: Handle| {
        kinds[count] = kind;
        items[count] = handle;
        count += 1;
    };
    add(
        boot_kind::PROCESS,
        put(Obj::Process(Arc::clone(process)), Rights::PROCESS)?,
    );
    add(
        boot_kind::VMAR_ROOT,
        put(Obj::Vmar(VmarObj::new(process)), Rights::VMAR)?,
    );
    add(
        boot_kind::CONSOLE,
        put(Obj::Console(ConsoleObj::new()), Rights::CONSOLE)?,
    );
    let initramfs = system().and_then(|system| {
        let (base, len) = system.initramfs?;
        options
            .initramfs
            .then(|| Vmo::physical(system.frames, base, len.div_ceil(PAGE_SIZE)))
    });
    if let Some(vmo) = initramfs {
        add(boot_kind::INITRAMFS, put(Obj::Vmo(vmo), Rights::INITRAMFS)?);
    }
    let mut format = None;
    if options.framebuffer
        && let Some((vmo, described)) =
            system().and_then(|system| framebuffer::boot_vmo(system.frames))
    {
        add(
            boot_kind::FRAMEBUFFER,
            put(Obj::Vmo(vmo), Rights::FRAMEBUFFER)?,
        );
        format = Some(described);
    }
    let mut message = [0_u8; 16 + 4 * ITEMS_MAX + FramebufferFormat::WIRE_BYTES];
    let len =
        encode_boot_message(&kinds[..count], format, &mut message).ok_or(Status::InvalidArgs)?;
    let mut work = Work::new();
    channel_write(
        &kernel,
        channel,
        Bytes::uncharged(&message[..len]).ok_or(Status::NoMemory)?,
        &items[..count],
        &mut work,
    )?;
    if !options.keep_peer
        && let Ok(transit) = kernel.with(|table| table.remove(channel))
    {
        close(transit, &mut work);
    }
    let boot = process
        .table()
        .with(|table| table.insert(receiver, Rights::CHANNEL))
        .map_err(|_| Status::NoMemory)?;
    let kept = options.keep_peer.then_some(BootPeer {
        table: kernel,
        channel,
    });
    Ok((boot.raw(), kept))
}
