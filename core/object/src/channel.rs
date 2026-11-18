//! O estado de um par de pontas de canal e a fila de cada uma (SDD 17, "Destruição de um canal"
//! e "As chamadas"). Puro: o kernel guarda o `Channel` atrás de **um lock por par**.

use core::fmt;

use rustos_hal::abi::{CHANNEL_MAX_BYTES, CHANNEL_MAX_HANDLES, CHANNEL_QUEUE_LEN, Status, signals};

use crate::object::{EndId, Object, Side, Transits, Worklist};

/// Uma mensagem: bytes já copiados para o kernel e os handles em trânsito.
pub struct Message<T: Object> {
    bytes: T::Bytes,
    handles: Transits<T>,
    seq: u32,
}

impl<T: Object> Message<T> {
    /// `InvalidArgs` se passa dos limites do canal (bytes ou handles); os handles voltam.
    pub fn new(bytes: T::Bytes, handles: Transits<T>) -> Result<Self, (Status, Transits<T>)> {
        if bytes.as_ref().len() > CHANNEL_MAX_BYTES || handles.len() > CHANNEL_MAX_HANDLES {
            return Err((Status::InvalidArgs, handles));
        }
        Ok(Self {
            bytes,
            handles,
            seq: 0,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    pub const fn handle_count(&self) -> usize {
        self.handles.len()
    }

    /// Os handles em trânsito: quem retira a mensagem os instala ou os fecha.
    pub fn into_parts(self) -> (T::Bytes, Transits<T>) {
        (self.bytes, self.handles)
    }
}

impl<T: Object> fmt::Debug for Message<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Message")
            .field("bytes", &self.bytes.as_ref().len())
            .field("handles", &self.handles.len())
            .field("seq", &self.seq)
            .finish()
    }
}

/// O que a leitura vê da mensagem da frente, sem retirá-la.
#[derive(Debug)]
pub struct Peek<T: Object> {
    /// O número de sequência: `pop_if` só retira se a frente ainda for esta mensagem.
    pub seq: u32,
    pub bytes: T::Bytes,
    pub handles: usize,
}

/// Um lugar reservado na fila da ponta oposta, ainda sem mensagem.
#[derive(Debug)]
#[must_use = "uma reserva de fila tem de ir para `commit` ou `cancel`"]
pub struct SendTicket {
    dest: Side,
}

struct End<T: Object> {
    queue: [Option<Message<T>>; CHANNEL_QUEUE_LEN],
    head: usize,
    len: usize,
    reserved: usize,
    /// Handles **desta** ponta que estão **na fila dela**: não a mantêm alcançável.
    own_in_queue: u32,
    dead: bool,
    next_seq: u32,
}

impl<T: Object> End<T> {
    const fn new() -> Self {
        Self {
            queue: [const { None }; CHANNEL_QUEUE_LEN],
            head: 0,
            len: 0,
            reserved: 0,
            own_in_queue: 0,
            dead: false,
            next_seq: 0,
        }
    }

    const fn has_room(&self) -> bool {
        self.len + self.reserved < CHANNEL_QUEUE_LEN
    }

    fn push(&mut self, mut message: Message<T>) {
        message.seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        let tail = (self.head + self.len) % CHANNEL_QUEUE_LEN;
        self.queue[tail] = Some(message);
        self.len += 1;
    }

    fn front(&self) -> Option<&Message<T>> {
        self.queue[self.head].as_ref()
    }

    fn pop(&mut self) -> Option<Message<T>> {
        let message = self.queue[self.head].take()?;
        self.head = (self.head + 1) % CHANNEL_QUEUE_LEN;
        self.len -= 1;
        Some(message)
    }
}

/// As duas pontas e as filas delas: a fila de uma ponta guarda o que a outra escreveu.
pub struct Channel<T: Object> {
    id: usize,
    ends: [End<T>; 2],
}

impl<T: Object> fmt::Debug for Channel<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Channel")
            .field("id", &self.id)
            .field("queued", &[self.ends[0].len, self.ends[1].len])
            .field("dead", &[self.ends[0].dead, self.ends[1].dead])
            .finish()
    }
}

impl<T: Object> Channel<T> {
    /// `id` é o número único do par (`EndId::channel`).
    pub const fn new(id: usize) -> Self {
        Self {
            id,
            ends: [End::new(), End::new()],
        }
    }

    pub const fn id(&self) -> usize {
        self.id
    }

    pub const fn end_id(&self, side: Side) -> EndId {
        EndId {
            channel: self.id,
            side,
        }
    }

    pub const fn is_dead(&self, side: Side) -> bool {
        self.ends[side.index()].dead
    }

    /// Mensagens na fila da ponta (as que a outra escreveu).
    pub const fn queued(&self, side: Side) -> usize {
        self.ends[side.index()].len
    }

    /// Handles da própria ponta que estão na fila dela.
    pub const fn own_in_queue(&self, side: Side) -> u32 {
        self.ends[side.index()].own_in_queue
    }

    /// Os sinais de `object_wait_one` da ponta, por nível. `PEER_CLOSED` não apaga `READABLE`:
    /// quem escreve e fecha ainda é lido.
    pub fn signals(&self, side: Side) -> u32 {
        let own = &self.ends[side.index()];
        let peer = &self.ends[side.other().index()];
        let mut bits = 0;
        if own.len > 0 {
            bits |= signals::READABLE;
        }
        if !peer.dead && peer.has_room() {
            bits |= signals::WRITABLE;
        }
        if peer.dead {
            bits |= signals::PEER_CLOSED;
        }
        bits
    }

    /// Passo 2 do `channel_write`: a ponta oposta está viva e há espaço? Reserva o lugar.
    /// `PeerClosed` consome o array do chamador; `ShouldWait` não.
    pub fn reserve(&mut self, from: Side) -> Result<SendTicket, Status> {
        let dest = &mut self.ends[from.other().index()];
        if dest.dead {
            return Err(Status::PeerClosed);
        }
        if !dest.has_room() {
            return Err(Status::ShouldWait);
        }
        dest.reserved += 1;
        Ok(SendTicket { dest: from.other() })
    }

    /// Solta a reserva sem mensagem.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "o ticket vale uma vez: é consumido"
    )]
    pub fn cancel(&mut self, ticket: SendTicket) {
        self.ends[ticket.dest.index()].reserved -= 1;
    }

    /// O *commit* da escrita: reconfere a ponta oposta **sob o lock do canal** (lacuna L1). Se
    /// ela morreu no meio, devolve a mensagem para o chamador fechar os handles. Se a mensagem
    /// leva um handle da ponta de destino e só restam esses, a ponta morre na própria escrita
    /// (`Ok(true)`).
    #[allow(
        clippy::needless_pass_by_value,
        reason = "o ticket vale uma vez: é consumido"
    )]
    pub fn commit<W: Worklist<T>>(
        &mut self,
        ticket: SendTicket,
        message: Message<T>,
        work: &mut W,
    ) -> Result<bool, Message<T>> {
        let dest = ticket.dest;
        let id = self.end_id(dest);
        let end = &mut self.ends[dest.index()];
        end.reserved -= 1;
        if end.dead {
            return Err(message);
        }
        let mut own_count = None;
        for transit in message.handles.iter() {
            if transit.object().channel_end() == Some(id) {
                end.own_in_queue += 1;
                own_count = Some(transit.object().handles().get());
            }
        }
        end.push(message);
        Ok(own_count.is_some_and(|count| self.on_handles_changed(dest, count, work)))
    }

    /// Passo 1 do `channel_read`: a mensagem da frente, sem retirá-la. Fila vazia: `PeerClosed`
    /// se a oposta morreu, senão `ShouldWait`.
    pub fn peek(&self, side: Side) -> Result<Peek<T>, Status> {
        match self.ends[side.index()].front() {
            Some(message) => Ok(Peek {
                seq: message.seq,
                bytes: message.bytes.clone(),
                handles: message.handles.len(),
            }),
            None if self.ends[side.other().index()].dead => Err(Status::PeerClosed),
            None => Err(Status::ShouldWait),
        }
    }

    /// Passo 5 do `channel_read`: retira a mensagem da frente se ela ainda é a `seq` que se
    /// espiou; `None` quer dizer que outra leitura passou na frente (repetir desde o passo 1).
    pub fn pop_if(&mut self, side: Side, seq: u32) -> Option<Message<T>> {
        let id = self.end_id(side);
        let end = &mut self.ends[side.index()];
        if end.front()?.seq != seq {
            return None;
        }
        let message = end.pop()?;
        for transit in message.handles.iter() {
            if transit.object().channel_end() == Some(id) {
                end.own_in_queue -= 1;
            }
        }
        Some(message)
    }

    /// Roda quando a contagem de handles da ponta muda (um fechamento, uma escrita): se só
    /// restam handles dentro da fila dela, a ponta é inalcançável e morre. `true` se morreu.
    pub fn on_handles_changed<W: Worklist<T>>(
        &mut self,
        side: Side,
        handle_count: u32,
        work: &mut W,
    ) -> bool {
        let end = &mut self.ends[side.index()];
        debug_assert!(handle_count >= end.own_in_queue);
        if end.dead || handle_count != end.own_in_queue {
            return false;
        }
        end.dead = true;
        end.own_in_queue = 0;
        while let Some(message) = end.pop() {
            message.handles.push_all(work);
        }
        true
    }
}
