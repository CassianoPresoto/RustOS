//! O relógio do boot: cada fase soma os ciclos do contador que gastou, e a conta vira
//! milissegundos só no fim, quando o x86_64 já mediu a frequência do TSC (SDD 04).

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::platform;

/// Em que o boot está gastando tempo. Só a CPU de boot troca de fase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Da entrada do `kmain` até a pilha própria: mapa, buddy, heap, tabelas de página.
    Memory,
    /// Serial e tela.
    Console,
    /// Exceções, controlador, calibração do timer e IRQs.
    Interrupts,
    /// Núcleos secundários, até todos ficarem online.
    Smp,
    /// Devolver a memória do bootloader.
    Reclaim,
    /// Os autotestes de corretude, com as janelas de espera deles.
    SelfTests,
    /// Medidas: `zero bench:`, `random bench:`, `switch:`, `scale:`, `alloc storm:`.
    Benches,
    /// Semente do gerador, linhas de estado e o escalonador.
    Setup,
    /// Carregar o `init` e esperar ele sair (só o modo de teste).
    Init,
}

const PHASES: usize = 9;

static SPENT: [AtomicU64; PHASES] = [const { AtomicU64::new(0) }; PHASES];
static SINCE: AtomicU64 = AtomicU64::new(0);
static CURRENT: AtomicUsize = AtomicUsize::new(Phase::Memory as usize);

/// Tempo por fase, em microssegundos.
#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "lido só pelo modo de teste")
)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Summary {
    pub total: u64,
    pub memory: u64,
    pub console: u64,
    pub interrupts: u64,
    pub smp: u64,
    pub reclaim: u64,
    pub self_tests: u64,
    pub benches: u64,
    pub setup: u64,
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "lido só pelo modo de teste")
    )]
    pub init: u64,
}

/// Começa a contar em `entry_cycles`, o contador lido na entrada do `kmain`.
pub fn start(entry_cycles: u64) {
    SINCE.store(entry_cycles, Ordering::Relaxed);
}

/// Passa à fase `next`: o tempo desde o último marco vai para a fase que acabou.
pub fn enter(next: Phase) {
    let now = platform::cycles();
    let spent = now.wrapping_sub(SINCE.swap(now, Ordering::Relaxed));
    let current = CURRENT.swap(next as usize, Ordering::Relaxed);
    SPENT[current].fetch_add(spent, Ordering::Relaxed);
}

/// Fecha a fase atual e converte tudo para microssegundos.
pub fn finish() -> Summary {
    enter(Phase::Setup);
    let us = |phase: Phase| {
        platform::cycles_to_ns(SPENT[phase as usize].load(Ordering::Relaxed)) / 1_000
    };
    let all: u64 = SPENT.iter().map(|cell| cell.load(Ordering::Relaxed)).sum();
    Summary {
        total: platform::cycles_to_ns(all) / 1_000,
        memory: us(Phase::Memory),
        console: us(Phase::Console),
        interrupts: us(Phase::Interrupts),
        smp: us(Phase::Smp),
        reclaim: us(Phase::Reclaim),
        self_tests: us(Phase::SelfTests),
        benches: us(Phase::Benches),
        setup: us(Phase::Setup),
        init: us(Phase::Init),
    }
}
