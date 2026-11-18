//! Pânico do kernel: o primeiro núcleo a entrar para os outros, toma o console e escreve;
//! os outros nunca mais escrevem na tela (SDD 15).

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

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
            nested();
            return;
        }
        stop_this_cpu();
    }
    DEPTH.store(1, Ordering::Relaxed);
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

/// A dona entrou em pânico de novo: toma o console de volta, e do terceiro relatório em
/// diante (pânico ou falta) só escreve uma linha fixa e para.
fn nested() {
    if DEPTH.fetch_add(1, Ordering::Relaxed) >= MAX_DEPTH {
        early::print_str("[rustos] panic: failed again while reporting a panic\n");
        platform::cpu().halt_forever()
    }
    // SAFETY: esta CPU é a dona do pânico: quem segurava o console era ela mesma.
    unsafe { console::break_locks() };
}

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    begin();
    console::print_panic(format_args!("[rustos] panic: {info}\n"));
    platform::cpu().halt_forever()
}

/// Relatórios que a dona tenta imprimir (o primeiro e um dentro dele); só ela os conta.
const MAX_DEPTH: u32 = 2;
static DEPTH: AtomicU32 = AtomicU32::new(0);

fn panicker(_: usize) {
    panic!("teste de pânico na CPU {}", percpu::current().index());
}

/// Cria uma tarefa presa a outra CPU que entra em pânico (`fault cpu`). Só volta (`false`) se
/// não há outra CPU ativa; depois do pânico o sistema inteiro para.
pub fn panic_on_other_cpu() -> bool {
    let here = percpu::pinned(PerCpu::index);
    let active = crate::task::active_cpus();
    let Some(other) = (0..rustos_hal::MAX_CPUS).find(|&cpu| cpu != here && active >> cpu & 1 != 0)
    else {
        return false;
    };
    let spawned = crate::task::spawn(crate::task::Spawn {
        name: "panicker",
        entry: panicker,
        arg: 0,
        simd: false,
        params: rustos_sched::SchedParams::fair(0),
        affinity: 1 << other,
    });
    if spawned.is_err() {
        return false;
    }
    loop {
        crate::task::sleep_until(platform::now_ns().unwrap_or(0) + 10_000_000);
    }
}

/// Teste do pânico: a mensagem do primeiro pânico entra em pânico ao ser impressa.
#[cfg(feature = "panic-tests")]
pub fn nested_test() -> ! {
    static ARMED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
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

/// Teste do pânico: uma tarefa por CPU ativa, todas entram em pânico juntas (a do shell também).
/// Só um relatório pode sair.
#[cfg(feature = "panic-tests")]
pub fn simultaneous_test() -> bool {
    use core::sync::atomic::AtomicU32;

    use rustos_sched::SchedParams;

    use crate::task::{self, Spawn};

    static READY: AtomicU32 = AtomicU32::new(0);
    static TOTAL: AtomicU32 = AtomicU32::new(0);

    fn together(_: usize) {
        READY.fetch_add(1, Ordering::AcqRel);
        while READY.load(Ordering::Acquire) < TOTAL.load(Ordering::Acquire) {
            core::hint::spin_loop();
        }
        panic!("simultaneous panic on cpu {}", percpu::current().index());
    }

    let here = percpu::pinned(PerCpu::index);
    let active = task::active_cpus();
    TOTAL.store(active.count_ones(), Ordering::Release);
    for cpu in (0..rustos_hal::MAX_CPUS).filter(|&cpu| cpu != here && active >> cpu & 1 != 0) {
        let spawned = task::spawn(Spawn {
            name: "panicker",
            entry: together,
            arg: 0,
            simd: false,
            params: SchedParams::fair(0),
            affinity: 1 << cpu,
        });
        if spawned.is_err() {
            return false;
        }
    }
    together(0);
    false
}

/// Teste do escalonador: cede a CPU com um `IrqLock` preso, e ele tem de entrar em pânico.
#[cfg(feature = "panic-tests")]
pub fn held_lock_test() -> bool {
    static HELD: crate::sync::IrqLock<()> = crate::sync::IrqLock::new(());
    let _held = HELD.lock();
    crate::task::yield_now();
    false
}
