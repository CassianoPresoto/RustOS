//! Shell embutido no kernel: edição de linha, histórico e comandos do sistema.
//! Fala com o sistema pela porta `Host`, que o kernel implementa.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

mod ansi;
mod commands;
mod decoder;
mod fmtutil;
mod history;
mod host;
mod line;
mod parse;
mod shell;
#[cfg(test)]
mod testing;

pub use host::{
    BenchReport, CpuInfo, DisplayInfo, FaultTest, Host, MemoryInfo, ScaleSample, StormSample,
    SystemInfo, TaskInfo, TaskState,
};
pub use shell::Shell;
