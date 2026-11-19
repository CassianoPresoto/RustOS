//! O que o processo recebe ao nascer (SDD 17, "O canal de boot"): o programa do initramfs ganha a
//! ponta de um canal com a mensagem de boot já escrita; o programa plano de teste, só o console.

use alloc::sync::Arc;

use rustos_hal::PAGE_SIZE;
use rustos_hal::abi::{
    CHANNEL_MAX_BYTES, CHANNEL_MAX_HANDLES, Handle, Rights, Status, boot_kind, encode_boot_message,
};
use rustos_object::{HandleTable, Lock, channel_read, channel_write, close};

use super::{Options, Process, Source, VmarObj, system};
use crate::object::{Bytes, ConsoleObj, Obj, Table, VecSlots, Work, new_channel};
use crate::sync::IrqLock;
use crate::vmo::Vmo;

/// Os itens da mensagem de boot, na ordem dos handles: o processo, o VMAR raiz e o console; o VMO do
/// initramfs, só para o `init`, vem por último.
const ITEMS_MAX: usize = 4;

/// A ponta do canal de boot que o kernel guarda (só o modo de teste): fala com o programa por uma
/// tabela de handles própria, como o pai de um processo falaria.
#[derive(Debug)]
pub struct BootPeer {
    table: IrqLock<Table>,
    channel: Handle,
}

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

/// Põe na tabela o que o `source` pede e devolve o valor de `rdi`/`x0` e, se `keep`, a ponta do
/// kernel do canal de boot.
pub fn handles_of(
    process: &Arc<Process>,
    source: Source<'_>,
    #[allow(unused_variables, reason = "só o programa plano de teste lê")] flat_arg0: u64,
    options: Options,
) -> Result<(u64, Option<BootPeer>), Status> {
    match source {
        Source::Elf(_) => boot_channel(process, options),
        #[cfg(feature = "selftest")]
        Source::Flat(_) => {
            let console = process
                .table()
                .with(|table| table.insert(Obj::Console(ConsoleObj::new()), Rights::CONSOLE))
                .map_err(|_| Status::NoMemory)?;
            let _ = console;
            Ok((flat_arg0, None))
        }
    }
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
        add(
            boot_kind::INITRAMFS,
            put(Obj::Vmo(vmo), Rights::INITRAMFS)?,
        );
    }
    let mut message = [0_u8; 16 + 4 * ITEMS_MAX];
    let len = encode_boot_message(&kinds[..count], &mut message).ok_or(Status::InvalidArgs)?;
    let mut work = Work::new();
    channel_write(
        &kernel,
        channel,
        Bytes::uncharged(&message[..len]).ok_or(Status::NoMemory)?,
        &items[..count],
        &mut work,
    )?;
    if !options.keep_peer && let Ok(transit) = kernel.with(|table| table.remove(channel)) {
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
