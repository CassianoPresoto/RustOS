//! O objeto de teste: um enum de seis `Arc`s, como o do kernel, e as pontas de canal de verdade.

use std::cell::Cell;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rustos_hal::abi::{Handle, Rights, Status};

use crate::{
    Channel, EndId, HandleCount, HandleTable, Lock, Object, ObjectKind, Side, Slot, SlotStorage,
    Transit, Worklist,
};

/// Objeto sem comportamento: só conta handles. `id` o identifica nos testes.
pub struct Plain {
    pub id: usize,
    pub count: HandleCount,
    pub zero_events: AtomicUsize,
}

pub struct EndObj {
    pub side: Side,
    pub channel: Arc<Mutex<Channel<TestObj>>>,
    pub channel_id: usize,
    pub count: HandleCount,
    pub peer_closed_calls: AtomicUsize,
}

/// Seis variantes, cada uma com um `Arc`: tag mais ponteiro, 16 bytes, como o enum do kernel.
#[derive(Clone)]
pub enum TestObj {
    Process(Arc<Plain>),
    Thread(Arc<Plain>),
    Vmo(Arc<Plain>),
    Vmar(Arc<Plain>),
    Channel(Arc<EndObj>),
    Console(Arc<Plain>),
}

impl fmt::Debug for TestObj {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.plain_ref() {
            Some(plain) => write!(f, "{:?}#{}", self.kind(), plain.id),
            None => write!(f, "{:?}", self.kind()),
        }
    }
}

impl TestObj {
    pub fn plain(kind: ObjectKind, id: usize) -> Self {
        let plain = Arc::new(Plain {
            id,
            count: HandleCount::new(0),
            zero_events: AtomicUsize::new(0),
        });
        match kind {
            ObjectKind::Process => Self::Process(plain),
            ObjectKind::Thread => Self::Thread(plain),
            ObjectKind::Vmo => Self::Vmo(plain),
            ObjectKind::Vmar => Self::Vmar(plain),
            ObjectKind::Console => Self::Console(plain),
            ObjectKind::Channel => panic!("canal não é plain"),
        }
    }

    pub fn plain_ref(&self) -> Option<&Arc<Plain>> {
        match self {
            Self::Process(p)
            | Self::Thread(p)
            | Self::Vmo(p)
            | Self::Vmar(p)
            | Self::Console(p) => Some(p),
            Self::Channel(_) => None,
        }
    }

    pub fn end_ref(&self) -> Option<&Arc<EndObj>> {
        match self {
            Self::Channel(end) => Some(end),
            _ => None,
        }
    }
}

static NEXT_CHANNEL: AtomicUsize = AtomicUsize::new(1);

/// As duas pontas de um canal novo, com `handle_count` zero (o `commit` as conta).
pub fn new_channel() -> (TestObj, TestObj) {
    let id = NEXT_CHANNEL.fetch_add(1, Ordering::Relaxed);
    let channel = Arc::new(Mutex::new(Channel::new(id)));
    let end = |side| {
        TestObj::Channel(Arc::new(EndObj {
            side,
            channel: Arc::clone(&channel),
            channel_id: id,
            count: HandleCount::new(0),
            peer_closed_calls: AtomicUsize::new(0),
        }))
    };
    (end(Side::A), end(Side::B))
}

impl Object for TestObj {
    type Bytes = Arc<[u8]>;

    fn kind(&self) -> ObjectKind {
        match self {
            Self::Process(_) => ObjectKind::Process,
            Self::Thread(_) => ObjectKind::Thread,
            Self::Vmo(_) => ObjectKind::Vmo,
            Self::Vmar(_) => ObjectKind::Vmar,
            Self::Channel(_) => ObjectKind::Channel,
            Self::Console(_) => ObjectKind::Console,
        }
    }

    fn handles(&self) -> &HandleCount {
        match self {
            Self::Channel(end) => &end.count,
            Self::Process(p)
            | Self::Thread(p)
            | Self::Vmo(p)
            | Self::Vmar(p)
            | Self::Console(p) => &p.count,
        }
    }

    fn channel_end(&self) -> Option<EndId> {
        self.end_ref().map(|end| EndId {
            channel: end.channel_id,
            side: end.side,
        })
    }

    fn with_channel<R>(&self, f: impl FnOnce(&mut Channel<Self>) -> R) -> Option<R> {
        let end = self.end_ref()?;
        let mut channel = end.channel.lock().unwrap();
        Some(f(&mut channel))
    }

    fn on_zero_handles<W: Worklist<Self>>(&self, _work: &mut W) {
        assert_eq!(
            lock_depth(),
            0,
            "on_zero_handles rodou com a tabela trancada"
        );
        if let Some(plain) = self.plain_ref() {
            plain.zero_events.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn on_end_died(&self, _dead: Side) {
        if let Some(end) = self.end_ref() {
            assert!(
                end.channel.try_lock().is_ok(),
                "gancho com o lock do par preso"
            );
            end.peer_closed_calls.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// A memória dos slots do teste: um `Vec` (o kernel usa o mesmo, com `try_reserve`).
#[derive(Default)]
pub struct VecSlots(pub Vec<Slot<TestObj>>);

impl SlotStorage<TestObj> for VecSlots {
    fn slots(&self) -> &[Slot<TestObj>] {
        &self.0
    }

    fn slots_mut(&mut self) -> &mut [Slot<TestObj>] {
        &mut self.0
    }

    fn try_push(&mut self, slot: Slot<TestObj>) -> bool {
        self.0.push(slot);
        true
    }
}

pub type Table = HandleTable<TestObj, VecSlots>;

pub fn table() -> Table {
    HandleTable::new(VecSlots::default())
}

thread_local! {
    static LOCK_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Quantos `Locked` esta thread tem trancados agora.
pub fn lock_depth() -> usize {
    LOCK_DEPTH.with(Cell::get)
}

/// O lock da tabela no teste: um `Mutex` que conta a profundidade, para provar que o fechamento
/// roda fora dele.
#[derive(Default)]
pub struct Locked<X>(Mutex<X>);

impl<X> Locked<X> {
    pub fn new(inner: X) -> Self {
        Self(Mutex::new(inner))
    }
}

impl<X> Lock<X> for Locked<X> {
    fn with<R>(&self, f: impl FnOnce(&mut X) -> R) -> R {
        let mut guard = self.0.lock().unwrap();
        LOCK_DEPTH.with(|depth| depth.set(depth.get() + 1));
        let result = f(&mut guard);
        LOCK_DEPTH.with(|depth| depth.set(depth.get() - 1));
        result
    }
}

pub type Tab = Locked<Table>;

pub fn tab() -> Tab {
    Locked::new(table())
}

/// A lista de trabalho do teste: uma pilha num `Vec`.
#[derive(Default)]
pub struct Stack(pub Vec<Transit<TestObj>>);

impl Worklist<TestObj> for Stack {
    fn push(&mut self, transit: Transit<TestObj>) {
        self.0.push(transit);
    }

    fn pop(&mut self) -> Option<Transit<TestObj>> {
        self.0.pop()
    }
}

/// Fecha o handle como o `handle_close` do kernel: remove sob o lock, fecha depois.
pub fn close_handle(table: &Tab, handle: Handle) -> Result<(), Status> {
    let transit = table.with(|t| t.remove(handle))?;
    let mut work = Stack::default();
    crate::close(transit, &mut work);
    Ok(())
}

pub fn handle_of(table: &Tab, object: &TestObj, rights: Rights) -> Handle {
    table
        .with(|t| t.insert(object.clone(), rights))
        .expect("tabela com lugar")
}

pub fn count(object: &TestObj) -> u32 {
    object.handles().get()
}

/// Escreve uma mensagem pelo `channel_write` real, com a pilha de trabalho do teste.
pub fn write(table: &Tab, channel: Handle, bytes: &[u8], handles: &[Handle]) -> Result<(), Status> {
    let mut work = Stack::default();
    crate::channel_write(table, channel, Arc::from(bytes), handles, &mut work)
}

/// Lê pelo `channel_read` real; devolve os bytes e os handles novos.
pub fn read(
    table: &Tab,
    channel: Handle,
    bytes_cap: usize,
    handles_cap: usize,
) -> Result<(Vec<u8>, Vec<Handle>), crate::ReadFailed> {
    let mut got = (Vec::new(), Vec::new());
    crate::channel_read(table, channel, bytes_cap, handles_cap, |bytes, handles| {
        got = (bytes.to_vec(), handles.to_vec());
        true
    })?;
    Ok(got)
}
