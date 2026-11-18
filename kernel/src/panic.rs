//! Pânico do kernel: o primeiro núcleo a entrar para os outros, toma o console e escreve;
//! os outros nunca mais escrevem na tela (SDD 15).

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use rustos_hal::{Cpu, Ipi, IpiKind};

use crate::percpu::PerCpu;
use crate::{console, early, percpu, platform};

/// Índice da CPU que entrou em pânico primeiro, mais um; zero enquanto ninguém entrou.
static OWNER: AtomicUsize = AtomicUsize::new(0);
/// CPUs que já pararam por causa do pânico de outra.
static STOPPED: AtomicU32 = AtomicU32::new(0);
/// Quanto o pânico espera as outras CPUs pararem: voltas de `spin_loop`, sem relógio.
const STOP_WAIT_SPINS: u32 = 50_000_000;

/// Algum núcleo entrou em pânico e não é este.
pub fn elsewhere() -> bool {
    let owner = OWNER.load(Ordering::Relaxed);
    owner != 0 && owner != percpu::pinned(PerCpu::index) + 1
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
    let me = percpu::pinned(PerCpu::index) + 1;
    if OWNER
        .compare_exchange(0, me, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        if OWNER.load(Ordering::Acquire) == me {
            // SAFETY: esta CPU é a dona do pânico: quem segurava o console era ela mesma.
            unsafe { console::break_locks() };
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
    if DEPTH.fetch_add(1, Ordering::Relaxed) >= MAX_DEPTH {
        early::print_str("[rustos] panic: panicked again while reporting a panic\n");
        platform::cpu().halt_forever()
    }
    begin();
    console::print_panic(format_args!("[rustos] panic: {info}\n"));
    platform::cpu().halt_forever()
}

/// Pânicos dentro do relatório de pânico que ainda se tentam imprimir; o seguinte só avisa.
const MAX_DEPTH: u32 = 2;
static DEPTH: AtomicU32 = AtomicU32::new(0);

/// Teste do pânico: a mensagem do primeiro pânico entra em pânico ao ser impressa.
pub fn nested_test() -> ! {
    static ARMED: AtomicBool = AtomicBool::new(false);
    struct Message;
    impl core::fmt::Display for Message {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            assert!(
                !ARMED.swap(false, Ordering::Relaxed),
                "nested: the message of the first panic panicked"
            );
            f.write_str("first")
        }
    }
    ARMED.store(true, Ordering::Relaxed);
    panic!("{}", Message)
}
