//! Escalonamento: identidade de tarefa, fila de execução e política.
//! Política pura; a troca de contexto é de `arch/`, ligada pelo kernel.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod queue;
pub mod round_robin;

pub use queue::{QueueFull, RunQueue};
pub use round_robin::RoundRobin;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(u32);

impl TaskId {
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn as_u32(self) -> u32 {
        self.0
    }
}
