//! Pânico do kernel: o primeiro núcleo a entrar para os outros, toma o console e escreve;
//! os outros nunca mais escrevem na tela (SDD 15).

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use rustos_hal::{Cpu, Ipi, IpiKind};

use crate::{console, percpu, platform};

/// Índice da CPU que entrou em pânico primeiro, mais um; zero enquanto ninguém entrou.
static OWNER: AtomicUsize = AtomicUsize::new(0);
/// CPUs que já pararam por causa do pânico de outra.
static STOPPED: AtomicU32 = AtomicU32::new(0);
/// Quanto o pânico espera as outras CPUs pararem: voltas de `spin_loop`, sem relógio.
const STOP_WAIT_SPINS: u32 = 50_000_000;

/// Algum núcleo entrou em pânico e não é este.
pub fn elsewhere() -> bool {
    let owner = OWNER.load(Ordering::Relaxed);
    owner != 0 && owner != percpu::current().index() + 1
}

/// Para esta CPU para sempre se outro núcleo entrou em pânico. Chamada onde a CPU gira
/// com a interrupção mascarada e não ouviria o IPI de parada.
pub fn stop_if_requested() {
    if elsewhere() {
        stop_this_cpu();
    }
}

/// Parada pedida pelo IPI ou pelo giro: conta e dorme, sem escrever em lugar nenhum.
pub fn stop_this_cpu() -> ! {
    STOPPED.fetch_add(1, Ordering::Release);
    platform::cpu().halt_forever()
}

/// Entra em pânico: o primeiro núcleo ganha o console, para os outros e toma os locks que
/// eles seguravam. O segundo a chegar aqui só para. Chamar antes de imprimir.
pub fn begin() {
    let me = percpu::current().index() + 1;
    if OWNER
        .compare_exchange(0, me, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        if OWNER.load(Ordering::Acquire) == me {
            return;
        }
        stop_this_cpu();
    }
    let others = u32::try_from(percpu::online_count() - 1).unwrap_or(0);
    if others > 0 {
        // SAFETY: há outras CPUs online, então a interface de interrupção desta está ligada.
        let ipi = unsafe { platform::ipi() };
        ipi.send_all_but_self(IpiKind::Stop);
        for _ in 0..STOP_WAIT_SPINS {
            if STOPPED.load(Ordering::Acquire) >= others {
                break;
            }
            core::hint::spin_loop();
        }
    }
    // SAFETY: as outras CPUs pararam ou foram avisadas e ficam sem escrever (`elsewhere`).
    unsafe { console::break_locks() };
}

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    begin();
    console::print_panic(format_args!("[rustos] panic: {info}\n"));
    platform::cpu().halt_forever()
}
