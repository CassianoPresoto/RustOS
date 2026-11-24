//! O `init`: o primeiro processo, lançado pelo kernel a partir do initramfs (SDD 17, fatia 4b). Só o
//! modo de teste o lança: o kernel espera ele sair e confere o código. A 4c lhe dá trabalho e o
//! move para o boot normal.

use rustos_hal::abi::event_kind;

use crate::frames::Frames;
use crate::loader::{self, LoadError};
use crate::process::{self, Options, Source};
use crate::{platform, task, vm::Vm};

/// O nome do arquivo no initramfs, sem `./`.
const NAME: &[u8] = b"init";
/// Quanto o kernel espera o `init` sair, e o passo da espera.
const WAIT_NS: u64 = 15_000_000_000;
const POLL_NS: u64 = 100_000;

/// Carrega o `init`, espera ele sair e confere que saiu com 0. Sem initramfs ou sem arquivo
/// `init`, o boot segue (`true`); arquivo que não carrega, falha de usuário ou código diferente
/// de 0 dão `false`.
pub fn run(vm: &Vm, frames: Frames, map_offset: u64) -> bool {
    let Some(archive) = vm.initramfs() else {
        kprintln!("[rustos] user: no initramfs, the boot goes on without init");
        return true;
    };
    let started = platform::cycles();
    let free_before = frames.free_frames();
    let spawned = loader::find(archive, NAME)
        .map_err(process::ProcessError::Load)
        .and_then(|file| {
            let options = Options {
                keep_peer: false,
                initramfs: true,
                framebuffer: false,
            };
            process::spawn_with(
                frames,
                map_offset,
                Source::Elf(file),
                0,
                options,
                "init",
                &[],
            )
            .map(|(spawned, _)| spawned)
        });
    let spawned = match spawned {
        Ok(spawned) => spawned,
        Err(process::ProcessError::Load(LoadError::Missing)) => {
            kprintln!("[rustos] user: no init in the initramfs, the boot goes on without it");
            return true;
        }
        Err(error) => {
            kprintln!("[rustos] user: init FAILED to load ({error:?})");
            return false;
        }
    };
    let used = free_before.saturating_sub(frames.free_frames()) as u64;
    kprintln!("[rustos] user: init loaded ({} segments)", spawned.segments);
    if !wait_for_exit(spawned.task) {
        kprintln!("[rustos] user: init FAILED (did not exit in time)");
        return false;
    }
    let exited_ok = process::last_event().is_some_and(|(pid, event)| {
        pid == spawned.pid && event.kind == event_kind::EXIT && event.code == 0
    });
    if !exited_ok {
        kprintln!("[rustos] user: init FAILED (stopped or exited with a code other than 0)");
        return false;
    }
    report_cost(started, used, &spawned);
    true
}

fn wait_for_exit(id: rustos_sched::TaskId) -> bool {
    let Some(start) = platform::now_ns() else {
        return false;
    };
    while !task::has_exited(id) {
        let Some(now) = platform::now_ns() else {
            return false;
        };
        if now > start + WAIT_NS {
            return false;
        }
        task::sleep_until(now + POLL_NS);
    }
    true
}

/// O custo do menor processo: o tempo do `cpio` até a entrada em usuário e até a primeira syscall,
/// e os frames que ele consome, por origem (SDD 07).
fn report_cost(started: u64, used: u64, spawned: &process::Spawned) {
    let (entry, first_syscall) = process::entry_and_first_syscall_cycles();
    let us = |cycles: u64| platform::cycles_to_ns(cycles.saturating_sub(started)) / 1000;
    let stats = spawned.stats;
    kprintln!(
        "[rustos] user: init load {} us to the entry, {} us to the first syscall; {} frames: {} data, {} tables, {} kernel (stack, fpu, heap)",
        us(entry),
        us(first_syscall),
        used,
        stats.data,
        stats.tables,
        used.saturating_sub(stats.data + stats.tables)
    );
    if let Some((_, end)) = process::last_stats() {
        kprintln!(
            "[rustos] user: init at exit: {} data, {} tables, {} of {} lazy pages backed",
            end.data,
            end.tables,
            end.lazy_backed,
            end.lazy_reserved
        );
    }
    kprintln!(
        "[rustos] user: stack top drawn over {} bits of page, image at a fixed address (no ASLR, DEBT-0083)",
        loader::stack_aslr_bits()
    );
}
