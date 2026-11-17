//! Escalonamento: política pura atrás do trait `Scheduler`; a troca de contexto
//! é de `arch/` e o kernel liga as duas (SDD 14).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod composite;
pub mod eevdf;
pub mod fifo;
pub mod queue;
pub mod round_robin;
pub mod scheduler;
pub mod weight;

mod table;
mod tree;

#[cfg(test)]
mod bench;
#[cfg(test)]
mod property;
#[cfg(test)]
mod scenarios;
#[cfg(test)]
mod sim;

pub use composite::{Composite, RtBudget};
pub use eevdf::{Eevdf, EevdfConfig};
pub use fifo::Fifo;
pub use queue::{QueueFull, RunQueue};
pub use round_robin::RoundRobin;
pub use scheduler::{SchedError, SchedParams, Scheduler};

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
