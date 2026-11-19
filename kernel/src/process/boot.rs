//! O que o processo recebe ao nascer (SDD 17, "O canal de boot"): o programa do initramfs ganha a
//! ponta de um canal com a mensagem de boot já escrita; o programa plano de teste, só o console.

use alloc::sync::Arc;

use rustos_hal::abi::{
    CHANNEL_MAX_BYTES, CHANNEL_MAX_HANDLES, Handle, Rights, Status, boot_kind, encode_boot_message,
};
use rustos_object::{HandleTable, Lock, channel_read, channel_write, close};

use super::Source;
use crate::object::{ConsoleObj, Obj, ProcessObj, Table, VecSlots, Work, new_channel};
use crate::sync::IrqLock;

/// Os itens da mensagem de boot, na ordem dos handles. O VMAR raiz e o VMO do initramfs entram
/// com `vmar_map` e `vmo_create`.
const ITEMS: [u32; 2] = [boot_kind::PROCESS, boot_kind::CONSOLE];

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
            Arc::from(bytes),
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
    object: &Arc<ProcessObj>,
    table: &IrqLock<Table>,
    source: Source<'_>,
    #[allow(unused_variables, reason = "só o programa plano de teste lê")] flat_arg0: u64,
    keep: bool,
) -> Result<(u64, Option<BootPeer>), Status> {
    match source {
        Source::Elf(_) => boot_channel(object, table, keep),
        #[cfg(feature = "selftest")]
        Source::Flat(_) => {
            let console = table
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
    object: &Arc<ProcessObj>,
    table: &IrqLock<Table>,
    keep: bool,
) -> Result<(u64, Option<BootPeer>), Status> {
    let (sender, receiver) = new_channel([None, None]);
    let kernel = IrqLock::new(HandleTable::new(VecSlots::default()));
    let put = |object: Obj, rights: Rights| {
        kernel
            .with(|table| table.insert(object, rights))
            .map_err(|_| Status::NoMemory)
    };
    let channel = put(sender, Rights::CHANNEL)?;
    let items = [
        put(Obj::Process(Arc::clone(object)), Rights::PROCESS)?,
        put(Obj::Console(ConsoleObj::new()), Rights::CONSOLE)?,
    ];
    debug_assert!(items.len() <= CHANNEL_MAX_HANDLES);
    let mut message = [0_u8; 16 + 4 * ITEMS.len()];
    let len = encode_boot_message(&ITEMS, &mut message).ok_or(Status::InvalidArgs)?;
    let mut work = Work::new();
    channel_write(
        &kernel,
        channel,
        Arc::from(&message[..len]),
        &items,
        &mut work,
    )?;
    if !keep && let Ok(transit) = kernel.with(|table| table.remove(channel)) {
        close(transit, &mut work);
    }
    let boot = table
        .with(|table| table.insert(receiver, Rights::CHANNEL))
        .map_err(|_| Status::NoMemory)?;
    let kept = keep.then_some(BootPeer {
        table: kernel,
        channel,
    });
    Ok((boot.raw(), kept))
}
