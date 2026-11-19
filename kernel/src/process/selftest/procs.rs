//! O programa `procs` do initramfs (SDD 17, fatia 4c, parte 3): os casos hostis de VMO, VMAR, processo e
//! thread, o teto em bytes e o laço de criar e matar processos. O kernel segura a outra ponta do
//! canal de boot e, nos pontos `A`, `B` e `C`, mede os frames, o heap e a promessa: de `A` a `C` nada
//! pode sobrar, nem frame, nem byte de heap, nem página prometida.

use super::super::{Options, Source, spawn_with};
use super::{WAIT_NS, exited_with, finish};
use crate::frames::Frames;
use crate::{commit, heap, loader, platform, process, task};

/// O que o kernel vê num ponto de sincronia.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sample {
    frames: usize,
    heap: u64,
    promised: u64,
}

fn sample(frames: Frames) -> Sample {
    Sample {
        frames: frames.free_frames(),
        heap: heap::stats().used_bytes,
        promised: commit::reserved(),
    }
}

/// Espera a mensagem `want` do programa e responde `go`, até `WAIT_NS`.
fn rendezvous(peer: &super::super::boot::BootPeer, want: &[u8], at: &mut Option<Sample>, frames: Frames) -> bool {
    let Some(start) = platform::now_ns() else {
        return false;
    };
    while platform::now_ns().is_some_and(|now| now < start + 4 * WAIT_NS) {
        if peer.read().is_ok_and(|got| got == want) {
            *at = Some(sample(frames));
            return peer.write(b"go").is_ok();
        }
        task::sleep_until(platform::now_ns().unwrap_or(0) + 1_000_000);
    }
    false
}

/// Roda o `procs` e confere o que ele diz e o que o kernel mede.
pub fn run(frames: Frames, map_offset: u64, archive: Option<&[u8]>) -> bool {
    let (Some(file), Some(_)) = (
        archive.and_then(|archive| loader::find(archive, b"procs").ok()),
        archive.and_then(|archive| loader::find(archive, b"child").ok()),
    ) else {
        return false;
    };
    let options = Options {
        keep_peer: true,
        initramfs: true,
    };
    let Ok((spawned, Some(peer))) = spawn_with(frames, map_offset, Source::Elf(file), 0, options)
    else {
        return false;
    };
    let (mut a, mut b, mut c) = (None, None, None);
    let synced = rendezvous(&peer, b"A", &mut a, frames)
        && rendezvous(&peer, b"B", &mut b, frames)
        && rendezvous(&peer, b"C", &mut c, frames);
    peer.close();
    let result = finish(&spawned);
    let (Some(a), Some(b), Some(c)) = (a, b, c) else {
        kprintln!("[rustos] user: procs FAILED (a rendezvous did not happen)");
        return false;
    };
    let exited = exited_with(result, 0);
    let exact = c == a;
    kprintln!(
        "[rustos] user: smallest child: {} frames ({} KiB) and {} B of heap while it waits",
        a.frames.saturating_sub(b.frames),
        a.frames.saturating_sub(b.frames) * 4,
        b.heap.saturating_sub(a.heap)
    );
    kprintln!(
        "[rustos] user: processes left {} frames, {} B of heap and {} promised pages after the loops",
        a.frames.abs_diff(c.frames),
        a.heap.abs_diff(c.heap),
        a.promised.abs_diff(c.promised)
    );
    let others = task::active_cpus().count_ones() < 2 || process::kill_cpus_max() >= 2;
    kprintln!(
        "[rustos] user: process_kill stopped threads on up to {} cpus at once",
        process::kill_cpus_max()
    );
    synced && exited && exact && others
}
