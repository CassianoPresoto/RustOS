//! A porta do escalonador: o kernel programa contra `Scheduler`, e trocar a
//! política não muda o kernel. Tempo é sempre `now_ns` monotônico, em ns.

use crate::TaskId;

/// Menor e maior prioridade de tempo real (`sched(7)`: 1 a 99).
pub const MIN_FIFO_PRIORITY: u8 = 1;
pub const MAX_FIFO_PRIORITY: u8 = 99;

/// Classe e parâmetros de uma tarefa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedParams {
    /// Classe justa. `slice_ns` é o pedido de fatia (`None`: a fatia base).
    Fair { nice: i8, slice_ns: Option<u64> },
    /// Tempo real, FIFO dentro da prioridade (1 a 99; maior vence).
    Fifo { priority: u8 },
}

impl SchedParams {
    pub const fn fair(nice: i8) -> Self {
        Self::Fair {
            nice,
            slice_ns: None,
        }
    }

    pub const fn fair_with_slice(nice: i8, slice_ns: u64) -> Self {
        Self::Fair {
            nice,
            slice_ns: Some(slice_ns),
        }
    }

    pub const fn fifo(priority: u8) -> Self {
        Self::Fifo { priority }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedError {
    /// Não há slot livre: a capacidade é fixa.
    Full,
    /// O `TaskId` já está no escalonador.
    Duplicate,
    /// A classe de `SchedParams` não é a que este escalonador atende.
    WrongClass,
    /// Prioridade de tempo real fora de 1..=99.
    InvalidPriority,
}

/// Política de escalonamento de uma CPU. Nenhum método aloca.
///
/// Tarefa nova entra pronta. `TaskId` desconhecido é ignorado em `remove`, `wake`
/// e `sleep`. Mexer na corrente só marca `needs_resched`: quem troca é o kernel.
pub trait Scheduler {
    /// Admite a tarefa, pronta para rodar.
    ///
    /// # Errors
    /// `Full`, `Duplicate`, `WrongClass` ou `InvalidPriority`.
    fn add(&mut self, task: TaskId, params: SchedParams, now_ns: u64) -> Result<(), SchedError>;

    /// A tarefa saiu do sistema, em qualquer estado.
    fn remove(&mut self, task: TaskId);

    /// A tarefa bloqueada ficou pronta.
    fn wake(&mut self, task: TaskId, now_ns: u64);

    /// A tarefa bloqueou (a corrente, em geral).
    fn sleep(&mut self, task: TaskId, now_ns: u64);

    /// Contabiliza a corrente, escolhe a próxima e a marca como corrente.
    /// `None`: nada pronto, a CPU fica ociosa.
    fn pick_next(&mut self, now_ns: u64) -> Option<TaskId>;

    /// Contabiliza a corrente até `now_ns`. `true`: o kernel deve chamar `pick_next`.
    fn tick(&mut self, now_ns: u64) -> bool;

    /// A corrente cede a vez.
    fn yield_current(&mut self, now_ns: u64);

    /// Quando o timer de um tiro deve disparar. `None`: nada o exige (sem tique).
    fn next_deadline_ns(&self, now_ns: u64) -> Option<u64>;

    /// Algum evento desde o último `pick_next` pede troca de tarefa.
    fn needs_resched(&self) -> bool;

    fn current(&self) -> Option<TaskId>;
}
