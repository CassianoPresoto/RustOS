//! Shell embutido no kernel: edição de linha, histórico e comandos do sistema.
//! Fala com o sistema pela porta `Host`, que o kernel implementa.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

mod ansi;
mod backlog;
#[cfg(feature = "bench")]
mod bench;
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

pub use backlog::Backlog;
pub use host::{
    Access, BenchReport, CpuInfo, DisplayInfo, FaultTest, Host, KillCause, KillOutcome, Listing,
    MemoryInfo, MicroBench, ProgramInfo, RunOutcome, ScaleSample, StormSample, SystemInfo,
    TaskInfo, TaskName, TaskState,
};
pub use shell::Shell;
