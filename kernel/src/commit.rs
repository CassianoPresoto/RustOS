//! Reserva estrita das páginas preguiçosas de usuário (SDD 17, decisão 9): a reserva de uma região
//! preguiçosa só passa se os frames livres cobrem tudo o que já foi prometido e ainda não ganhou
//! frame. A falta de memória vira erro na criação do processo, não morte na falta de página.

use core::sync::atomic::{AtomicU64, Ordering};

/// Páginas prometidas a regiões preguiçosas que ainda não ganharam frame.
static RESERVED: AtomicU64 = AtomicU64::new(0);

/// Promete `pages` páginas se os `free` frames livres cobrem o que já está prometido mais elas.
/// Só conta o usuário: o que o kernel aloca (heap, pilhas, tabelas) não entra na promessa.
pub fn reserve(pages: u64, free: u64) -> bool {
    RESERVED
        .try_update(Ordering::AcqRel, Ordering::Acquire, |held| {
            held.checked_add(pages).filter(|total| *total <= free)
        })
        .is_ok()
}

/// Devolve `pages` da promessa: a página ganhou frame ou o espaço morreu.
pub fn release(pages: u64) {
    let before = RESERVED.fetch_sub(pages, Ordering::AcqRel);
    debug_assert!(before >= pages, "reserva devolvida a mais");
}

/// Páginas prometidas e ainda sem frame.
#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "lido só pelo modo de teste")
)]
pub fn reserved() -> u64 {
    RESERVED.load(Ordering::Acquire)
}
