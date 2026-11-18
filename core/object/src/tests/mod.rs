//! Testes de host do `core/object`: o objeto de teste (`support`), as regras de recusa, a fila
//! do canal, as propriedades com semente fixa contra um modelo ingênuo, threads e a medida.

#![allow(
    clippy::cast_possible_truncation,
    clippy::result_large_err,
    clippy::std_instead_of_core,
    reason = "teste de host: valores pequenos, `std` disponível, sem alvo de 32 bits"
)]

mod bench;
mod channel;
mod model;
mod rights;
mod support;
mod table;
mod threads;
