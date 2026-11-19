//! A tabela de handles de um processo (SDD 17, "Tabela de handles" e "Operações sobre handle").
//! O valor do handle é `geração << 16 | (índice + 1)`; o slot tem 32 bytes com um `T` de 16.

use core::fmt;
use core::marker::PhantomData;

use rustos_hal::abi::{CHANNEL_MAX_HANDLES, Handle, MAX_HANDLES, Rights, Status};

use crate::object::{EndId, Object, ObjectKind, Transit, Transits, Worklist};
use crate::rights;

const NIL: u32 = u32::MAX;
const STATE_SHIFT: u32 = 48;
const GENERATION_MASK: u64 = (1 << STATE_SHIFT) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Free,
    Reserved,
    Used,
    Retired,
}

impl State {
    const fn from_bits(bits: u64) -> Self {
        match bits {
            0 => Self::Free,
            1 => Self::Reserved,
            2 => Self::Used,
            _ => Self::Retired,
        }
    }
}

/// Uma posição da tabela. O estado mora nos 16 bits altos da geração; `next` é o elo da lista
/// livre. Com um `T` que tem nicho (um enum de `Arc`s), o slot fecha em 32 bytes.
#[derive(Debug)]
pub struct Slot<T> {
    meta: u64,
    rights: Rights,
    next: u32,
    object: Option<T>,
}

impl<T> Slot<T> {
    /// Um slot nunca usado: livre, geração 0.
    pub const fn vacant() -> Self {
        Self {
            meta: 0,
            rights: Rights::NONE,
            next: NIL,
            object: None,
        }
    }

    const fn state(&self) -> State {
        State::from_bits(self.meta >> STATE_SHIFT)
    }

    const fn generation(&self) -> u64 {
        self.meta & GENERATION_MASK
    }

    const fn set(&mut self, state: State, generation: u64) {
        self.meta = (state as u64) << STATE_SHIFT | generation;
    }
}

/// A memória dos slots, que o kernel entrega: a tabela cresce por demanda e `core/` não chama o
/// alocador global (guidelines 05). O kernel implementa sobre um `Vec` com `try_reserve`.
pub trait SlotStorage<T> {
    fn slots(&self) -> &[Slot<T>];
    fn slots_mut(&mut self) -> &mut [Slot<T>];
    /// Acrescenta um slot; `false` se faltou memória (a tabela responde `NoMemory`).
    fn try_push(&mut self, slot: Slot<T>) -> bool;
    /// O único ponto de clamp do índice do handle contra o Spectre v1 (SDD 17, mitigações). Roda
    /// depois da conferência `index < len`. O kernel o sobrescreve com o clamp de `arch/`.
    fn clamp_index(&self, index: usize, len: usize) -> usize {
        rustos_hal::nospec::array_index_nospec(index, len)
    }
}

/// Capacidade fixa, sem alocação: serve a tabelas pequenas e ao teste.
pub struct FixedSlots<T, const N: usize> {
    slots: [Slot<T>; N],
    len: usize,
}

impl<T, const N: usize> FixedSlots<T, N> {
    pub const fn new() -> Self {
        Self {
            slots: [const { Slot::vacant() }; N],
            len: 0,
        }
    }
}

impl<T, const N: usize> Default for FixedSlots<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> fmt::Debug for FixedSlots<T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FixedSlots")
            .field("len", &self.len)
            .field("capacity", &N)
            .finish_non_exhaustive()
    }
}

impl<T, const N: usize> SlotStorage<T> for FixedSlots<T, N> {
    fn slots(&self) -> &[Slot<T>] {
        &self.slots[..self.len]
    }

    fn slots_mut(&mut self) -> &mut [Slot<T>] {
        &mut self.slots[..self.len]
    }

    fn try_push(&mut self, slot: Slot<T>) -> bool {
        if self.len == N {
            return false;
        }
        self.slots[self.len] = slot;
        self.len += 1;
        true
    }
}

/// Os tetos da tabela. `ABI` é o contrato; o teste baixa a geração para esgotá-la.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Handles em tabela (usados mais reservados); passar disso é `NoMemory`.
    pub max_handles: usize,
    /// O slot cuja próxima geração chegaria a este valor é aposentado.
    pub max_generation: u64,
}

impl Limits {
    pub const ABI: Self = Self {
        max_handles: MAX_HANDLES,
        max_generation: Handle::MAX_GENERATION,
    };
}

/// Um slot tirado da lista livre, com o valor do handle que ele vai ter. Ou vira handle
/// (`commit`, `commit_moved`) ou se desfaz (`cancel`); o valor reservado e solto não volta.
#[derive(Debug)]
#[must_use = "uma reserva tem de ir para `commit`, `commit_moved` ou `cancel`"]
pub struct Reservation {
    handle: Handle,
}

impl Reservation {
    pub const fn handle(&self) -> Handle {
        self.handle
    }
}

/// O que `get` achou: o objeto e os direitos do handle.
#[derive(Debug)]
pub struct Granted<'a, T> {
    pub object: &'a T,
    pub rights: Rights,
}

/// `replace` falhou. O handle velho **já foi invalidado** (verificado no Zircon); se existia, vem
/// em `closed` para ser fechado fora do lock.
#[derive(Debug)]
#[must_use = "o handle invalidado em `closed` tem de ser fechado"]
pub struct ReplaceFailed<T> {
    pub status: Status,
    pub closed: Option<Transit<T>>,
}

/// `take_array` falhou; os handles do array que existiam saíram da tabela e vêm em `taken`.
#[derive(Debug)]
#[must_use = "os handles em `taken` têm de ser fechados"]
pub struct TakeFailed<T> {
    pub status: Status,
    pub taken: Transits<T>,
}

/// A tabela de handles. Um `IrqLock` por processo a protege; aqui nada tranca.
pub struct HandleTable<T, S> {
    slots: S,
    free_head: u32,
    free_tail: u32,
    used: usize,
    reserved: usize,
    retired: usize,
    limits: Limits,
    marker: PhantomData<fn() -> T>,
}

impl<T, S> fmt::Debug for HandleTable<T, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandleTable")
            .field("used", &self.used)
            .field("reserved", &self.reserved)
            .field("retired", &self.retired)
            .finish_non_exhaustive()
    }
}

/// O valor cru que o usuário passou, como handle: `0` e o que não tem forma de handle é `BadHandle`.
pub const fn handle_from_raw(raw: u64) -> Result<Handle, Status> {
    match Handle::from_raw(raw) {
        Some(handle) => Ok(handle),
        None => Err(Status::BadHandle),
    }
}

fn link(index: usize) -> u32 {
    u32::try_from(index).expect("o índice cabe em 16 bits")
}

impl<T: Object, S: SlotStorage<T>> HandleTable<T, S> {
    /// Uma tabela com os tetos da ABI, sobre uma memória vazia.
    pub fn new(storage: S) -> Self {
        Self::with_limits(storage, Limits::ABI)
    }

    /// # Panics
    /// Se a memória não está vazia ou a geração máxima está fora de `1..=MAX_GENERATION`.
    pub fn with_limits(storage: S, limits: Limits) -> Self {
        assert!(storage.slots().is_empty(), "a tabela nasce sem slots");
        assert!(
            (1..=Handle::MAX_GENERATION).contains(&limits.max_generation),
            "geração máxima fora de 1..=2^48-1"
        );
        Self {
            slots: storage,
            free_head: NIL,
            free_tail: NIL,
            used: 0,
            reserved: 0,
            retired: 0,
            limits,
            marker: PhantomData,
        }
    }

    /// A memória dos slots, para quem a entregou conferir o que ela guarda.
    pub const fn storage(&self) -> &S {
        &self.slots
    }

    /// Handles vivos (sem contar as reservas).
    pub const fn len(&self) -> usize {
        self.used
    }

    pub const fn is_empty(&self) -> bool {
        self.used == 0
    }

    pub const fn reserved(&self) -> usize {
        self.reserved
    }

    /// Slots que a tabela já criou (inclusive os aposentados).
    pub fn slot_count(&self) -> usize {
        self.slots.slots().len()
    }

    /// Slots que chegaram ao limite da geração e nunca mais voltam.
    pub const fn retired(&self) -> usize {
        self.retired
    }

    fn locate(&self, handle: Handle) -> Result<usize, Status> {
        if handle.raw().trailing_zeros() >= 16 {
            return Err(Status::BadHandle);
        }
        let slots = self.slots.slots();
        if handle.index() >= slots.len() {
            return Err(Status::BadHandle);
        }
        let index = self.slots.clamp_index(handle.index(), slots.len());
        let slot = &slots[index];
        if slot.state() == State::Used && slot.generation() == handle.generation() {
            Ok(index)
        } else {
            Err(Status::BadHandle)
        }
    }

    /// O handle existe e tem o que a operação exige.
    pub fn get(&self, handle: Handle, required: Rights) -> Result<Granted<'_, T>, Status> {
        let index = self.locate(handle)?;
        let slot = &self.slots.slots()[index];
        rights::require(slot.rights, required)?;
        let object = slot.object.as_ref().ok_or(Status::BadHandle)?;
        Ok(Granted {
            object,
            rights: slot.rights,
        })
    }

    /// Como `get`, e o objeto é do tipo pedido (`WrongType`, antes de conferir os direitos).
    pub fn get_of(
        &self,
        handle: Handle,
        kind: ObjectKind,
        required: Rights,
    ) -> Result<Granted<'_, T>, Status> {
        let index = self.locate(handle)?;
        let slot = &self.slots.slots()[index];
        let object = slot.object.as_ref().ok_or(Status::BadHandle)?;
        if object.kind() != kind {
            return Err(Status::WrongType);
        }
        rights::require(slot.rights, required)?;
        Ok(Granted {
            object,
            rights: slot.rights,
        })
    }

    fn take_slot(&mut self) -> Result<usize, Status> {
        if self.used + self.reserved >= self.limits.max_handles {
            return Err(Status::NoMemory);
        }
        let index = if self.free_head == NIL {
            let len = self.slots.slots().len();
            if len >= Handle::MAX_SLOTS || !self.slots.try_push(Slot::vacant()) {
                return Err(Status::NoMemory);
            }
            len
        } else {
            let index = self.free_head as usize;
            self.free_head = self.slots.slots()[index].next;
            if self.free_head == NIL {
                self.free_tail = NIL;
            }
            index
        };
        let slot = &mut self.slots.slots_mut()[index];
        slot.next = NIL;
        let generation = slot.generation();
        slot.set(State::Reserved, generation);
        self.reserved += 1;
        Ok(index)
    }

    /// Devolve o slot: a geração sobe e ele vai para o fim da lista livre; se a próxima
    /// geração chegaria ao limite, o slot é aposentado e nunca mais é usado.
    fn release_slot(&mut self, index: usize) {
        let max = self.limits.max_generation;
        let slot = &mut self.slots.slots_mut()[index];
        slot.object = None;
        slot.rights = Rights::NONE;
        let current = slot.generation();
        if current + 1 >= max {
            slot.set(State::Retired, current);
            self.retired += 1;
            return;
        }
        slot.set(State::Free, current + 1);
        slot.next = NIL;
        let this = link(index);
        if self.free_tail == NIL {
            self.free_head = this;
        } else {
            self.slots.slots_mut()[self.free_tail as usize].next = this;
        }
        self.free_tail = this;
    }

    /// Reserva um slot e diz o valor do handle que ele vai ter. `NoMemory` no teto de handles
    /// ou sem memória. Até o `commit`, o valor não é válido em nenhuma busca.
    pub fn reserve(&mut self) -> Result<Reservation, Status> {
        let index = self.take_slot()?;
        let generation = self.slots.slots()[index].generation();
        let handle = Handle::new(generation, index).ok_or(Status::NoMemory)?;
        Ok(Reservation { handle })
    }

    fn fill(&mut self, reservation: &Reservation, object: T, rights: Rights) -> Handle {
        let handle = reservation.handle;
        debug_assert!(
            rights.is_subset_of(object.kind().rights()),
            "direito que o tipo do objeto não tem"
        );
        let slot = &mut self.slots.slots_mut()[handle.index()];
        assert!(
            slot.state() == State::Reserved && slot.generation() == handle.generation(),
            "reserva de outra tabela ou já usada"
        );
        slot.object = Some(object);
        slot.rights = rights;
        slot.set(State::Used, handle.generation());
        self.reserved -= 1;
        self.used += 1;
        handle
    }

    /// Põe um objeto novo no slot reservado: o handle conta em `handle_count`.
    ///
    /// # Panics
    /// Se a reserva não é desta tabela (conferido por índice e geração).
    #[allow(
        clippy::needless_pass_by_value,
        reason = "a reserva vale uma vez: é consumida"
    )]
    pub fn commit(&mut self, reservation: Reservation, object: T, rights: Rights) -> Handle {
        object.handles().acquire();
        self.fill(&reservation, object, rights)
    }

    /// Põe um handle que veio de uma mensagem ou de outra tabela: a contagem não muda.
    ///
    /// # Panics
    /// Se a reserva não é desta tabela.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "a reserva vale uma vez: é consumida"
    )]
    pub fn commit_moved(&mut self, reservation: Reservation, transit: Transit<T>) -> Handle {
        let (object, rights) = transit.into_inner();
        self.fill(&reservation, object, rights)
    }

    /// Desfaz a reserva sem deixar handle: o valor reservado nunca mais é válido.
    ///
    /// # Panics
    /// Se a reserva não é desta tabela.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "a reserva vale uma vez: é consumida"
    )]
    pub fn cancel(&mut self, reservation: Reservation) {
        let handle = reservation.handle;
        let slot = &self.slots.slots()[handle.index()];
        assert!(
            slot.state() == State::Reserved && slot.generation() == handle.generation(),
            "reserva de outra tabela ou já usada"
        );
        self.reserved -= 1;
        self.release_slot(handle.index());
    }

    /// Objeto novo na tabela. Sem lugar, devolve o objeto (a contagem não chegou a subir).
    pub fn insert(&mut self, object: T, rights: Rights) -> Result<Handle, T> {
        match self.reserve() {
            Ok(reservation) => Ok(self.commit(reservation, object, rights)),
            Err(_) => Err(object),
        }
    }

    fn take_used(&mut self, index: usize) -> Transit<T> {
        let slot = &mut self.slots.slots_mut()[index];
        let rights = slot.rights;
        let object = slot.object.take().expect("slot em uso tem objeto");
        self.used -= 1;
        self.release_slot(index);
        Transit::new(object, rights)
    }

    /// Valida e remove num passo só (`handle_close`). Nenhum direito é exigido. O handle sai
    /// da tabela mas ainda conta: o chamador o passa a `close`, depois de soltar o lock.
    pub fn remove(&mut self, handle: Handle) -> Result<Transit<T>, Status> {
        let index = self.locate(handle)?;
        Ok(self.take_used(index))
    }

    /// `handle_duplicate`: `DUPLICATE` no handle, direitos novos dentro dos dele.
    pub fn duplicate(&mut self, handle: Handle, wanted: Rights) -> Result<Handle, Status>
    where
        T: Clone,
    {
        let index = self.locate(handle)?;
        let slot = &self.slots.slots()[index];
        rights::require(slot.rights, rights::required::HANDLE_DUPLICATE)?;
        let wanted = rights::reduce(slot.rights, wanted)?;
        let object = slot.object.clone().ok_or(Status::BadHandle)?;
        self.insert(object, wanted).map_err(|_| Status::NoMemory)
    }

    /// `handle_replace`: o velho sai **sempre** que existe, em sucesso e em erro; nenhum direito
    /// é exigido. A contagem do objeto não muda no sucesso.
    pub fn replace(&mut self, handle: Handle, wanted: Rights) -> Result<Handle, ReplaceFailed<T>> {
        let Ok(index) = self.locate(handle) else {
            return Err(ReplaceFailed {
                status: Status::BadHandle,
                closed: None,
            });
        };
        let old = self.take_used(index);
        if let Err(status) = rights::reduce(old.rights(), wanted) {
            return Err(ReplaceFailed {
                status,
                closed: Some(old),
            });
        }
        match self.reserve() {
            Ok(reservation) => {
                let (object, _) = old.into_inner();
                Ok(self.commit_moved(reservation, Transit::new(object, wanted)))
            }
            Err(status) => Err(ReplaceFailed {
                status,
                closed: Some(old),
            }),
        }
    }

    /// Confere um array de handles que vai se mover (`channel_write`, `process_create`): cada
    /// um existe, não se repete, tem o direito do par e não é a ponta `reject`. Só lê.
    pub fn check_array(
        &self,
        items: &[(Handle, Rights)],
        reject: Option<EndId>,
    ) -> Result<(), Status> {
        if items.len() > CHANNEL_MAX_HANDLES {
            return Err(Status::InvalidArgs);
        }
        for (position, &(handle, required)) in items.iter().enumerate() {
            let index = self.locate(handle)?;
            if items[..position]
                .iter()
                .any(|&(earlier, _)| earlier == handle)
            {
                return Err(Status::BadHandle);
            }
            let slot = &self.slots.slots()[index];
            rights::require(slot.rights, required)?;
            let end = slot.object.as_ref().and_then(Object::channel_end);
            if reject.is_some() && end == reject {
                return Err(Status::NotSupported);
            }
        }
        Ok(())
    }

    /// Tira da tabela, de uma vez, todo handle do array que existe (o primeiro de cada valor).
    pub fn remove_existing(&mut self, handles: &[Handle]) -> Transits<T> {
        let mut taken = Transits::new();
        for &handle in handles.iter().take(CHANNEL_MAX_HANDLES) {
            if let Ok(transit) = self.remove(handle) {
                taken.push(transit);
            }
        }
        taken
    }

    /// `check_array` e depois a remoção de todos; em erro, os que existiam saem do mesmo jeito
    /// (a regra geral de quem move handle) e vêm em `taken` para serem fechados.
    pub fn take_array(
        &mut self,
        items: &[(Handle, Rights)],
        reject: Option<EndId>,
    ) -> Result<Transits<T>, TakeFailed<T>> {
        let verdict = self.check_array(items, reject);
        let mut taken = Transits::new();
        if items.len() > CHANNEL_MAX_HANDLES {
            return Err(TakeFailed {
                status: Status::InvalidArgs,
                taken,
            });
        }
        for &(handle, _) in items {
            if let Ok(transit) = self.remove(handle) {
                taken.push(transit);
            }
        }
        match verdict {
            Ok(()) => Ok(taken),
            Err(status) => Err(TakeFailed { status, taken }),
        }
    }

    /// A morte do processo: todo handle vivo vai para a lista de trabalho, para `close_pending`.
    pub fn drain<W: Worklist<T>>(&mut self, work: &mut W) {
        for index in 0..self.slots.slots().len() {
            if self.slots.slots()[index].state() == State::Used {
                let transit = self.take_used(index);
                work.push(transit);
            }
        }
    }

    /// Leva a geração de um slot livre a um valor, para o teste não girar 2^48 vezes.
    #[cfg(test)]
    pub(crate) fn set_generation(&mut self, index: usize, generation: u64) {
        let slot = &mut self.slots.slots_mut()[index];
        assert_eq!(slot.state(), State::Free);
        slot.set(State::Free, generation);
    }
}
