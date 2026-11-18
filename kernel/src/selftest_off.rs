//! A imagem enxuta, sem a feature `selftest`: o modo de teste não existe e o `rustos.selftest=1`
//! é recusado numa linha (SDD 05). Cada função faz par com a de `selftest.rs`.

use rustos_hal::BootInfo;
use rustos_shell::DisplayInfo;

use crate::frames::Frames;
use crate::memory::Memory;
use crate::platform::FpuSummary;
use crate::{random, vm};

/// Recusa o boot de teste pedido: esta imagem não o tem, e o boot segue no modo normal.
pub fn enabled(requested: bool) -> bool {
    if requested {
        kprintln!("[rustos] selftest: not in this build");
    }
    false
}

pub fn details(
    _: &impl BootInfo,
    _: &Memory,
    _: Option<DisplayInfo>,
    _: &vm::Vm,
    _: random::Source,
) {
}

pub fn initramfs(_: &vm::Vm) -> bool {
    true
}

pub fn before_tasks() -> bool {
    true
}

pub fn after_tasks(_: FpuSummary, _: Frames, _: u64, _: &vm::Vm) -> bool {
    true
}

pub fn boot_phases(_: &crate::boot_time::Summary) {}
