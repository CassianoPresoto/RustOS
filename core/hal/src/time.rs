//! Porta de tempo: relógio monotônico desde um instante qualquer do boot.

pub trait Clock {
    /// Nanossegundos, sem voltar para trás; a origem é fixa durante o boot.
    fn now_ns(&self) -> u64;
}
