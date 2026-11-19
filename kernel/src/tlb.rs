//! *Shootdown* de TLB (SDD 17): invalida uma faixa de páginas de usuário nas outras CPUs que usam o
//! espaço. No x86_64 é um IPI com uma caixa por par (destino, origem) e espera de confirmação; no
//! aarch64 o `tlbi ...is` de `MmuControl::entry_removed` já alcança todas as CPUs e não há o que fazer.

#[cfg(target_arch = "x86_64")]
mod mailbox {
    use core::sync::atomic::{AtomicBool, AtomicU64, Ordering, fence};

    use rustos_hal::{Cpu, MAX_CPUS};

    use crate::sync::bump;
    use crate::{percpu, platform};

    /// Quanto uma CPU espera as confirmações antes de dar o IPI por perdido.
    const ACK_TIMEOUT_NS: u64 = 5_000_000_000;
    const ACK_SPIN_LIMIT: u64 = 2_000_000_000;

    /// O pedido de uma CPU de origem a uma de destino: só a origem escreve `start` e `pages` e
    /// sobe `pending`; só o destino o baixa, depois de invalidar.
    struct Request {
        start: AtomicU64,
        pages: AtomicU64,
        pending: AtomicBool,
    }

    static MAILBOX: [[Request; MAX_CPUS]; MAX_CPUS] = [const {
        [const {
            Request {
                start: AtomicU64::new(0),
                pages: AtomicU64::new(0),
                pending: AtomicBool::new(false),
            }
        }; MAX_CPUS]
    }; MAX_CPUS];

    /// Invalida `[start, start + pages)` nesta CPU e nas de `targets` (menos esta e as offline) e só
    /// volta quando todas confirmaram. O chamador já gravou a entrada nova ou removeu a velha.
    pub fn shoot(start: u64, pages: u64, targets: u32) {
        let were_enabled = platform::cpu().save_and_disable_interrupts();
        let me = percpu::current().index();
        let targets = targets & !(1 << me) & percpu::online_mask();
        if targets != 0 {
            fence(Ordering::SeqCst);
            send_all(me, start, pages, targets);
            wait_all(me, targets);
        }
        platform::cpu().restore_interrupts(were_enabled);
    }

    fn send_all(me: usize, start: u64, pages: u64, targets: u32) {
        for target in (0..MAX_CPUS).filter(|cpu| targets >> cpu & 1 != 0) {
            let slot = &MAILBOX[target][me];
            slot.start.store(start, Ordering::Relaxed);
            slot.pages.store(pages, Ordering::Relaxed);
            slot.pending.store(true, Ordering::Release);
        }
        for target in (0..MAX_CPUS).filter(|cpu| targets >> cpu & 1 != 0) {
            platform::send_shootdown(percpu::get(target).hw_id());
            bump(&percpu::current().stats.ipis_sent, 1);
        }
    }

    /// Espera as confirmações. Atende os pedidos que chegam a esta CPU enquanto isso: duas CPUs que
    /// se invalidam ao mesmo tempo, com a IRQ mascarada, não se travam.
    fn wait_all(me: usize, targets: u32) {
        let deadline = platform::now_ns().map(|now| now + ACK_TIMEOUT_NS);
        let mut spins = 0_u64;
        while (0..MAX_CPUS)
            .filter(|cpu| targets >> cpu & 1 != 0)
            .any(|cpu| MAILBOX[cpu][me].pending.load(Ordering::Acquire))
        {
            service();
            spins += 1;
            let late = deadline
                .zip(platform::now_ns())
                .is_some_and(|(limit, now)| now > limit);
            assert!(
                !late && spins < ACK_SPIN_LIMIT,
                "tlb shootdown sem resposta"
            );
            core::hint::spin_loop();
        }
    }

    /// Atende os pedidos de invalidação endereçados a esta CPU. Chamado pela IRQ do IPI e pelas
    /// esperas que giram com a IRQ mascarada.
    pub fn service() {
        let were_enabled = platform::cpu().save_and_disable_interrupts();
        let me = percpu::current().index();
        for slot in &MAILBOX[me] {
            if slot.pending.load(Ordering::Acquire) {
                platform::flush_pages(
                    slot.start.load(Ordering::Relaxed),
                    slot.pages.load(Ordering::Relaxed),
                );
                bump(&percpu::current().stats.ipis_received, 1);
                slot.pending.store(false, Ordering::Release);
            }
        }
        platform::cpu().restore_interrupts(were_enabled);
    }
}

#[cfg(target_arch = "x86_64")]
pub use mailbox::{service, shoot};

/// O `tlbi ...is` já invalidou em todas as CPUs: não há IPI nem espera.
#[cfg(target_arch = "aarch64")]
#[allow(
    clippy::missing_const_for_fn,
    reason = "mesma assinatura do x86_64, que não é const"
)]
pub fn shoot(_start: u64, _pages: u64, _targets: u32) {}

/// Sem IPI, nada a atender.
#[cfg(target_arch = "aarch64")]
#[allow(
    clippy::missing_const_for_fn,
    reason = "mesma assinatura do x86_64, que não é const"
)]
pub fn service() {}

/// Quando falso, o modo de teste desliga a parte remota da invalidação: a prova de que o teste de
/// *shootdown* enxerga a tradução velha que ela remove.
#[cfg(feature = "selftest")]
pub fn set_remote(on: bool) {
    #[cfg(target_arch = "x86_64")]
    REMOTE.store(on, core::sync::atomic::Ordering::Relaxed);
    #[cfg(target_arch = "aarch64")]
    crate::platform::set_local_only(!on);
}

#[cfg(all(feature = "selftest", target_arch = "x86_64"))]
static REMOTE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// A parte remota está ligada (sempre, fora do modo de teste).
#[cfg(target_arch = "x86_64")]
fn remote_on() -> bool {
    #[cfg(feature = "selftest")]
    return REMOTE.load(core::sync::atomic::Ordering::Relaxed);
    #[cfg(not(feature = "selftest"))]
    true
}

/// Invalida a página de `virt` nas CPUs de `targets`: o que o espaço de usuário chama depois de
/// remover uma entrada ou tirar permissão dela.
pub fn shoot_page(virt: u64, targets: u32) {
    shoot_range(virt, 1, targets);
}

/// Invalida `pages` páginas a partir de `virt` nas CPUs de `targets`, com um IPI só (acima de 32
/// páginas o destino reescreve o `CR3`).
pub fn shoot_range(virt: u64, pages: u64, targets: u32) {
    #[cfg(target_arch = "x86_64")]
    if remote_on() {
        shoot(virt, pages, targets);
    }
    #[cfg(target_arch = "aarch64")]
    shoot(virt, pages, targets);
}
