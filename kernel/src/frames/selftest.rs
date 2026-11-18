//! Ajudas do modo de teste para o alocador de frames, só com a feature `selftest`.

use core::sync::atomic::Ordering;

use rustos_hal::PhysAddr;
use rustos_mm::PhysFrame;

use super::{CACHE_ON, FRAMES, Frames};

/// Liga ou desliga o cache por CPU. Desligar não devolve o que ele guarda: `flush_caches`.
pub fn set_cache(on: bool) {
    CACHE_ON.store(on, Ordering::Release);
}

/// Devolve um frame que `allocate_frame` entregou.
pub fn free_frame(frames: Frames, frame: PhysAddr) {
    frames.deallocate(PhysFrame::containing(frame)).ok();
}

/// O handle, se o alocador já foi instalado.
pub fn handle() -> Option<Frames> {
    FRAMES.lock().is_some().then_some(Frames { _installed: () })
}

impl Frames {
    #[allow(
        clippy::unused_self,
        reason = "o handle prova que o alocador foi instalado"
    )]
    pub fn total_frames(self) -> usize {
        Self::with(|allocator| allocator.total_frames())
    }
}
