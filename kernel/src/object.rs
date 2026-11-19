//! Os objetos do kernel sobre o `core/object` (SDD 17, fatia 4c): o enum fechado de `Arc`s, o `Lock`
//! do `IrqLock`, a memória dos slots e a lista de trabalho. Ordem de lock: tabela, par de pontas,
//! fila do escalonador; e tabela, heap, frames (o `try_reserve` dos slots). Nenhum outro se toma.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering, fence};

use rustos_hal::abi::{MAX_CHANNEL_ENDS, signals};
use rustos_object::{
    Channel, EndId, HandleCount, HandleTable, Lock, Object, ObjectKind, Side, Slot, SlotStorage,
    Transit, Worklist,
};

use crate::sync::IrqLock;
use crate::{platform, task};

/// Os bytes de uma mensagem: copiados do usuário uma vez, e só lidos depois.
pub type Bytes = Arc<[u8]>;

/// O objeto que um handle aponta. Tag e ponteiro dão 16 bytes, e o slot da tabela, 32. `Vmo`, `Vmar`
/// e `Thread` entram com `vmo_create`, `vmar_map` e `thread_create`.
#[derive(Clone)]
pub enum Obj {
    Process(Arc<ProcessObj>),
    Channel(Arc<ChannelEnd>),
    Console(Arc<ConsoleObj>),
}

impl fmt::Debug for Obj {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Obj::{:?}", self.kind())
    }
}

impl<X> Lock<X> for IrqLock<X> {
    fn with<R>(&self, f: impl FnOnce(&mut X) -> R) -> R {
        f(&mut self.lock())
    }
}

/// A tabela de handles de um processo.
pub type Table = HandleTable<Obj, VecSlots>;

/// Os slots da tabela num `Vec` que cresce por demanda com `try_reserve`: falta de memória é
/// `NoMemory`, não aborta.
#[derive(Debug, Default)]
pub struct VecSlots(Vec<Slot<Obj>>);

impl SlotStorage<Obj> for VecSlots {
    fn slots(&self) -> &[Slot<Obj>] {
        &self.0
    }

    fn slots_mut(&mut self) -> &mut [Slot<Obj>] {
        &mut self.0
    }

    fn try_push(&mut self, slot: Slot<Obj>) -> bool {
        if self.0.try_reserve(1).is_err() {
            return false;
        }
        self.0.push(slot);
        true
    }

    fn clamp_index(&self, index: usize, len: usize) -> usize {
        platform::array_index_nospec(index, len)
    }
}

/// Quem espera um sinal do objeto: um bit por slot de tarefa. Sem alocação, sem lista, e acordar
/// roda com qualquer lock preso.
#[derive(Debug, Default)]
pub struct Waiters(AtomicU64);

impl Waiters {
    pub const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// Registra a tarefa corrente. Depois disto, quem confere o sinal de novo não perde o aviso.
    pub fn register(&self) -> u64 {
        let bit = task::wait_bit();
        self.0.fetch_or(bit, Ordering::SeqCst);
        bit
    }

    pub fn unregister(&self, bit: u64) {
        self.0.fetch_and(!bit, Ordering::SeqCst);
    }

    /// O sinal mudou: acorda quem se registrou. O `fence` separa a mudança (sob o lock do objeto)
    /// da leitura da máscara; o registro é uma operação `SeqCst`.
    pub fn wake(&self) {
        fence(Ordering::SeqCst);
        if self.0.load(Ordering::SeqCst) != 0 {
            task::wake_waiters(self.0.swap(0, Ordering::SeqCst));
        }
    }
}

/// Um processo como objeto: o que os outros esperam (`TERMINATED`) e a contagem de handles.
#[derive(Debug)]
pub struct ProcessObj {
    handles: HandleCount,
    terminated: AtomicBool,
    waiters: Waiters,
}

impl ProcessObj {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            handles: HandleCount::new(0),
            terminated: AtomicBool::new(false),
            waiters: Waiters::new(),
        })
    }

    /// O processo terminou: levanta `TERMINATED` e acorda quem espera.
    pub fn terminate(&self) {
        self.terminated.store(true, Ordering::SeqCst);
        self.waiters.wake();
    }
}

/// O console de depuração: só se escreve nele.
#[derive(Debug)]
pub struct ConsoleObj {
    handles: HandleCount,
}

impl ConsoleObj {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            handles: HandleCount::new(0),
        })
    }
}

/// Pontas de canal cobradas a um processo (`MAX_CHANNEL_ENDS`): a ponta é cobrada ao criador
/// enquanto existir, esteja o handle onde estiver.
#[derive(Debug, Default)]
pub struct Quota(AtomicUsize);

/// Uma ponta cobrada: devolve a cobrança ao ser solta.
#[derive(Debug)]
pub struct Charge(Arc<Quota>);

impl Charge {
    /// `None` se a conta passaria de `MAX_CHANNEL_ENDS`.
    pub fn new(quota: &Arc<Quota>) -> Option<Self> {
        quota
            .0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used < MAX_CHANNEL_ENDS).then_some(used + 1)
            })
            .ok()?;
        Some(Self(Arc::clone(quota)))
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.0.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// O par de pontas: um lock para as duas filas e a espera de cada ponta.
#[derive(Debug)]
pub struct Pair {
    channel: IrqLock<Channel<Obj>>,
    waiters: [Waiters; 2],
}

/// O par vazio de que todo par nasce por cópia: o par mede 7 KiB, e construí-lo na pilha de 16 KiB
/// da tarefa a estoura (DEBT-0091). Nunca é trancado nem mexido.
static EMPTY_PAIR: Pair = Pair {
    channel: IrqLock::new(Channel::new(0)),
    waiters: [Waiters::new(), Waiters::new()],
};

/// Um par vazio no heap, sem passar pela pilha.
fn empty_pair() -> Arc<Pair> {
    let mut pair = Arc::<Pair>::new_uninit();
    let slot = Arc::get_mut(&mut pair).expect("recém-criado, sem outro dono");
    // SAFETY: `EMPTY_PAIR` é um par sem mensagem, nunca trancado: não tem dono de recurso, então a
    // cópia bit a bit é um par vazio válido; `slot` tem o tamanho e o alinhamento de `Pair`.
    unsafe { core::ptr::copy_nonoverlapping(&raw const EMPTY_PAIR, slot.as_mut_ptr(), 1) };
    // SAFETY: o par foi inicializado pela cópia acima.
    unsafe { pair.assume_init() }
}

/// O número do próximo par: 64 bits, que não dão a volta (o kernel só existe em 64 bits).
static NEXT_CHANNEL_ID: AtomicUsize = AtomicUsize::new(1);

impl Pair {
    /// Roda `f` sob o lock do par e, solto o lock, acorda a ponta cujo sinal mudou.
    fn with<R>(&self, f: impl FnOnce(&mut Channel<Obj>) -> R) -> R {
        let (result, changed) = {
            let mut channel = self.channel.lock();
            let before = [channel.signals(Side::A), channel.signals(Side::B)];
            let result = f(&mut channel);
            let after = [channel.signals(Side::A), channel.signals(Side::B)];
            (result, [before[0] != after[0], before[1] != after[1]])
        };
        for (waiters, changed) in self.waiters.iter().zip(changed) {
            if changed {
                waiters.wake();
            }
        }
        result
    }
}

/// Uma ponta de canal.
#[derive(Debug)]
pub struct ChannelEnd {
    handles: HandleCount,
    /// O número do par, o mesmo do `Channel`: `channel_end` o lê sem lock.
    id: usize,
    side: Side,
    pair: Arc<Pair>,
    #[allow(dead_code, reason = "só o `Drop` da cobrança importa")]
    charge: Option<Charge>,
}

/// As duas pontas de um canal novo. `charges` é o que se cobra de cada ponta: nenhuma no canal que
/// o próprio kernel cria.
pub fn new_channel(charges: [Option<Charge>; 2]) -> (Obj, Obj) {
    let id = NEXT_CHANNEL_ID.fetch_add(1, Ordering::Relaxed);
    let pair = empty_pair();
    pair.channel.lock().assign_id(id);
    let [charge_a, charge_b] = charges;
    let end = |side, charge| {
        Obj::Channel(Arc::new(ChannelEnd {
            handles: HandleCount::new(0),
            id,
            side,
            pair: Arc::clone(&pair),
            charge,
        }))
    };
    (end(Side::A, charge_a), end(Side::B, charge_b))
}

impl Obj {
    /// Os sinais de `object_wait_one` agora, por nível.
    pub fn signals(&self) -> u32 {
        match self {
            Self::Process(process) => {
                if process.terminated.load(Ordering::SeqCst) {
                    signals::TERMINATED
                } else {
                    0
                }
            }
            Self::Channel(end) => end.pair.channel.lock().signals(end.side),
            Self::Console(_) => 0,
        }
    }

    /// Onde registrar uma espera por sinal deste objeto.
    pub fn waiters(&self) -> Option<&Waiters> {
        match self {
            Self::Process(process) => Some(&process.waiters),
            Self::Channel(end) => Some(&end.pair.waiters[end.side as usize]),
            Self::Console(_) => None,
        }
    }
}

impl Object for Obj {
    type Bytes = Bytes;

    fn kind(&self) -> ObjectKind {
        match self {
            Self::Process(_) => ObjectKind::Process,
            Self::Channel(_) => ObjectKind::Channel,
            Self::Console(_) => ObjectKind::Console,
        }
    }

    fn handles(&self) -> &HandleCount {
        match self {
            Self::Process(process) => &process.handles,
            Self::Channel(end) => &end.handles,
            Self::Console(console) => &console.handles,
        }
    }

    fn channel_end(&self) -> Option<EndId> {
        match self {
            Self::Channel(end) => Some(EndId {
                channel: end.id,
                side: end.side,
            }),
            _ => None,
        }
    }

    fn with_channel<R>(&self, f: impl FnOnce(&mut Channel<Self>) -> R) -> Option<R> {
        match self {
            Self::Channel(end) => Some(end.pair.with(f)),
            _ => None,
        }
    }
}

/// A lista de trabalho do fechamento: uma pilha num `Vec`. Nunca recebe `push` com lock preso
/// (`drain_dead` e o fechamento rodam soltos), então pode crescer.
#[derive(Debug, Default)]
pub struct Work(Vec<Transit<Obj>>);

impl Work {
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// Reserva `count` posições antes de pegar um lock, para `drain` não alocar com ele preso.
    pub fn reserve(&mut self, count: usize) {
        self.0.reserve(count);
    }
}

impl Worklist<Obj> for Work {
    fn push(&mut self, transit: Transit<Obj>) {
        self.0.push(transit);
    }

    fn pop(&mut self) -> Option<Transit<Obj>> {
        self.0.pop()
    }
}
