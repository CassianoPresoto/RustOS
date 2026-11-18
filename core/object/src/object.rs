//! O que o kernel precisa implementar para um objeto entrar na tabela, a contagem de handles
//! e o fechamento em laço, sem recursão (SDD 17, "Tabela de handles" e "Destruição de um canal").

use core::sync::atomic::{AtomicU32, Ordering};

use rustos_hal::abi::{CHANNEL_MAX_HANDLES, Rights};

use crate::channel::Channel;

/// Os tipos de objeto da fase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Process,
    Thread,
    Vmo,
    Vmar,
    Channel,
    Console,
}

impl ObjectKind {
    /// Os direitos que fazem sentido para o tipo (a tabela "Direitos iniciais" do SDD 17).
    pub const fn rights(self) -> Rights {
        match self {
            Self::Process => Rights::PROCESS,
            Self::Thread => Rights::THREAD,
            Self::Vmo => Rights::VMO,
            Self::Vmar => Rights::VMAR,
            Self::Channel => Rights::CHANNEL,
            Self::Console => Rights::CONSOLE,
        }
    }
}

/// Uma das duas pontas de um canal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

impl Side {
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }

    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

/// A identidade de uma ponta: o canal (um número único, como o endereço do par) e o lado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndId {
    pub channel: usize,
    pub side: Side,
}

/// Quantos handles apontam para um objeto: os da tabela e os em trânsito numa mensagem.
#[derive(Debug, Default)]
pub struct HandleCount(AtomicU32);

impl HandleCount {
    pub const fn new(count: u32) -> Self {
        Self(AtomicU32::new(count))
    }

    pub fn get(&self) -> u32 {
        self.0.load(Ordering::Acquire)
    }

    /// Nasceu um handle para o objeto.
    ///
    /// # Panics
    /// Se a contagem transbordaria `u32`.
    pub fn acquire(&self) {
        let before = self.0.fetch_add(1, Ordering::Relaxed);
        assert!(before < u32::MAX, "contagem de handles transbordou");
    }

    /// Um handle deixou de existir; devolve quantos restam.
    ///
    /// # Panics
    /// Se a contagem já era zero: um handle foi fechado duas vezes.
    pub fn release(&self) -> u32 {
        let before = self.0.fetch_sub(1, Ordering::AcqRel);
        assert!(before > 0, "handle fechado a mais");
        before - 1
    }
}

/// Um handle fora da tabela, a caminho de uma mensagem ou de ser fechado. Ainda conta na
/// `handle_count` do objeto: só `close` o desconta, então não se solta sem fechar.
#[derive(Debug)]
#[must_use = "um handle em trânsito tem de ir para uma mensagem, uma tabela ou `close`"]
pub struct Transit<T> {
    object: T,
    rights: Rights,
}

impl<T> Transit<T> {
    pub(crate) const fn new(object: T, rights: Rights) -> Self {
        Self { object, rights }
    }

    pub(crate) fn into_inner(self) -> (T, Rights) {
        (self.object, self.rights)
    }

    pub const fn object(&self) -> &T {
        &self.object
    }

    pub const fn rights(&self) -> Rights {
        self.rights
    }
}

/// Até `CHANNEL_MAX_HANDLES` handles em trânsito, sem alocar.
#[derive(Debug)]
#[must_use = "os handles em trânsito têm de ir para uma mensagem ou `close_all`"]
pub struct Transits<T> {
    items: [Option<Transit<T>>; CHANNEL_MAX_HANDLES],
    len: usize,
}

impl<T> Transits<T> {
    pub const fn new() -> Self {
        Self {
            items: [const { None }; CHANNEL_MAX_HANDLES],
            len: 0,
        }
    }

    pub(crate) fn push(&mut self, transit: Transit<T>) {
        assert!(self.len < CHANNEL_MAX_HANDLES, "mais handles que o teto");
        self.items[self.len] = Some(transit);
        self.len += 1;
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = &Transit<T>> {
        self.items[..self.len].iter().flatten()
    }

    pub(crate) fn take(&mut self, index: usize) -> Option<Transit<T>> {
        self.items.get_mut(index)?.take()
    }
}

impl<T> Transits<T> {
    pub(crate) fn push_all<W: Worklist<T>>(mut self, work: &mut W) {
        for index in 0..self.len {
            if let Some(transit) = self.take(index) {
                work.push(transit);
            }
        }
    }
}

impl<T: Object> Transits<T> {
    /// Fecha todos, em laço e sem recursão. Roda **sem** o lock da tabela.
    pub fn close_all<W: Worklist<T>>(mut self, work: &mut W) {
        for index in 0..self.len {
            if let Some(transit) = self.take(index) {
                close(transit, work);
            }
        }
    }
}

impl<T> Default for Transits<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// A lista de trabalho do fechamento: o que o kernel entrega para o laço não recursar.
/// `push` não falha; quem implementa decide onde guardar (SDD 17, decisões do agente).
pub trait Worklist<T> {
    fn push(&mut self, transit: Transit<T>);
    fn pop(&mut self) -> Option<Transit<T>>;
}

/// Um lock que o kernel tem de implementar sobre o seu `IrqLock`: a ordem é tabela, depois canal.
pub trait Lock<X> {
    fn with<R>(&self, f: impl FnOnce(&mut X) -> R) -> R;
}

/// O que o objeto do kernel (o `T` da tabela, um enum de `Arc`s) oferece à tabela.
pub trait Object: Sized {
    /// Os bytes de uma mensagem, já copiados para o kernel; `Clone` barato (um `Arc<[u8]>`).
    type Bytes: AsRef<[u8]> + Clone;

    fn kind(&self) -> ObjectKind;

    fn handles(&self) -> &HandleCount;

    /// A ponta de canal que o objeto é; `None` para os outros tipos. Sem lock.
    fn channel_end(&self) -> Option<EndId>;

    /// Roda `f` com o lock do par de pontas; `None` se o objeto não é uma ponta de canal.
    fn with_channel<R>(&self, f: impl FnOnce(&mut Channel<Self>) -> R) -> Option<R>;

    /// Chamado por `close` depois de descontar um handle, **sem** lock nenhum. Numa ponta de
    /// canal, a ponta morre quando só restam handles dentro da fila dela; a contagem é lida de
    /// novo **sob o lock do par**, porque `remaining` já pode estar velho.
    fn on_handle_closed<W: Worklist<Self>>(&self, remaining: u32, work: &mut W) {
        if let Some(end) = self.channel_end() {
            let died = self
                .with_channel(|channel| {
                    channel.on_handles_changed(end.side, self.handles().get(), work)
                })
                .unwrap_or(false);
            if died {
                self.on_end_died(end.side);
            }
        } else if remaining == 0 {
            self.on_zero_handles(work);
        }
    }

    /// Process, Thread, VMO e Console: nada a fazer por padrão (o kernel os solta com o `Arc`).
    fn on_zero_handles<W: Worklist<Self>>(&self, _work: &mut W) {}

    /// A ponta `dead` do par morreu: o kernel acorda quem espera `PEER_CLOSED` na outra. Roda
    /// sem o lock do par; na escrita, ainda com o lock da tabela de quem escreveu.
    fn on_end_died(&self, _dead: Side) {}
}

/// Fecha um handle e tudo o que a morte dele alcançar, até a lista esvaziar.
pub fn close<T: Object, W: Worklist<T>>(first: Transit<T>, work: &mut W) {
    let mut current = Some(first);
    while let Some(transit) = current {
        let remaining = transit.object.handles().release();
        transit.object.on_handle_closed(remaining, work);
        current = work.pop();
    }
}

/// Fecha o que sobrou na lista (depois de `HandleTable::drain`, por exemplo).
pub fn close_pending<T: Object, W: Worklist<T>>(work: &mut W) {
    if let Some(first) = work.pop() {
        close(first, work);
    }
}
