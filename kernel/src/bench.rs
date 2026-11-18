//! Medidas e autoteste das tarefas: latência de troca de contexto na mesma CPU e entre CPUs
//! (o equivalente do `lat_ctx` do lmbench), IPI de ida e volta, escala com o número de
//! núcleos, jitter de acordar e o estado de SIMD por tarefa.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use rustos_hal::{Ipi, IpiKind};
use rustos_sched::{Composite, SchedParams, Scheduler, TaskId};
use rustos_shell::{BenchReport, ScaleSample, StormSample};

use crate::task::{self, Spawn};
use crate::{interrupts, percpu, platform};

/// Quanto cada medida repete.
#[derive(Clone, Copy, Debug)]
pub struct Effort {
    pub rounds: u32,
    pub samples: u32,
    /// CPU que cada tarefa da escala gasta, em ms.
    pub task_ms: u64,
    /// Voltas de alocação de cada CPU na medida de contenção da memória.
    pub storm: u32,
    /// Quanto a CPU gira antes da primeira medida (ver `warm_up`).
    pub warmup_ms: u64,
}

/// Amostras do autoteste de acordar entre tarefas.
const WAKE_SELF_TEST_SAMPLES: u32 = 20;

/// O acordar entre CPUs custa dezenas de vezes o da mesma CPU: menos voltas para a medida caber.
const CROSS_DIVISOR: u32 = 20;

/// Tarefas de CPU na medida de escala: o dobro das CPUs do maior caso (4).
const SCALE_TASKS: usize = 8;

/// O curto do boot e o completo do comando `bench`.
pub const BOOT: Effort = Effort {
    rounds: 2_000,
    samples: 20,
    task_ms: 2,
    storm: 2_000,
    warmup_ms: 0,
};
pub const FULL: Effort = Effort {
    rounds: 20_000,
    samples: 200,
    task_ms: 10,
    storm: 20_000,
    warmup_ms: 300,
};

static PING: AtomicUsize = AtomicUsize::new(0);
static PONG: AtomicUsize = AtomicUsize::new(0);
static MAIN: AtomicUsize = AtomicUsize::new(0);
static STOP: AtomicBool = AtomicBool::new(false);
static ROUNDS: AtomicU32 = AtomicU32::new(0);
static ELAPSED_NS: AtomicU64 = AtomicU64::new(0);
/// Tarefas de medida que terminaram a função delas; quem as criou espera por todas.
static FINISHED: AtomicU32 = AtomicU32::new(0);

fn finish() {
    FINISHED.fetch_add(1, Ordering::Release);
    task::unpark(id(&MAIN));
}

fn wait_finished(count: u32) {
    while FINISHED.load(Ordering::Acquire) < count {
        task::park();
    }
}

/// O slot das tarefas só volta ao `spawn` depois de `Exited`; esperar por isso mantém os
/// slots iguais de uma rodada à outra.
fn wait_exited(ids: &[TaskId]) {
    while !ids.iter().all(|&id| task::has_exited(id)) {
        task::yield_now();
    }
}

fn id(slot: &AtomicUsize) -> TaskId {
    TaskId::new(u32::try_from(slot.load(Ordering::Relaxed)).unwrap_or(0))
}

fn ping(_: usize) {
    PING.store(task::current().as_u32() as usize, Ordering::Release);
    let pong = id(&PONG);
    let start = platform::cycles();
    for _ in 0..ROUNDS.load(Ordering::Relaxed) {
        task::unpark(pong);
        task::park();
    }
    let elapsed = platform::cycles().wrapping_sub(start);
    ELAPSED_NS.store(platform::cycles_to_ns(elapsed), Ordering::Relaxed);
    STOP.store(true, Ordering::Release);
    task::unpark(pong);
    finish();
}

fn pong(_: usize) {
    task::park();
    let ping = id(&PING);
    loop {
        if STOP.load(Ordering::Acquire) {
            break;
        }
        task::unpark(ping);
        task::park();
    }
    finish();
}

/// Onde as duas tarefas do ping-pong rodam.
#[derive(Clone, Copy, Debug)]
pub enum Placement {
    /// As duas na CPU de quem mede.
    SameCpu,
    /// Uma na CPU de quem mede e a outra numa segunda CPU ativa.
    CrossCpu,
}

/// A CPU de quem mede e, se houver, outra ativa.
fn cpu_pair() -> (usize, Option<usize>) {
    let here = percpu::pinned(percpu::PerCpu::index);
    let other = (0..rustos_hal::MAX_CPUS)
        .find(|&index| index != here && task::active_cpus() >> index & 1 != 0);
    (here, other)
}

/// Nanossegundos por troca de contexto: um aquecimento e o melhor de três rodadas.
/// `None` se a colocação pede uma segunda CPU que não existe.
pub fn context_switch_ns(rounds: u32, simd: bool, placement: Placement) -> Option<u64> {
    ping_pong(rounds, simd, placement)?;
    let mut best = u64::MAX;
    for _ in 0..3 {
        best = best.min(ping_pong(rounds, simd, placement)?);
    }
    Some(best)
}

/// Duas tarefas passam a vez uma à outra por `unpark` e `park`; cada ida e volta
/// são duas trocas.
fn ping_pong(rounds: u32, simd: bool, placement: Placement) -> Option<u64> {
    let (here, other) = cpu_pair();
    let (first_on, second_on) = match placement {
        Placement::SameCpu => (1 << here, 1 << here),
        Placement::CrossCpu => (1 << here, 1 << other?),
    };
    STOP.store(false, Ordering::Relaxed);
    FINISHED.store(0, Ordering::Relaxed);
    ROUNDS.store(rounds, Ordering::Relaxed);
    MAIN.store(task::current().as_u32() as usize, Ordering::Relaxed);
    let spawn = |name, entry, affinity| Spawn {
        name,
        entry,
        arg: 0,
        simd,
        params: SchedParams::fair(0),
        affinity,
    };
    let second = task::spawn(spawn("pong", pong, second_on)).ok()?;
    PONG.store(second.as_u32() as usize, Ordering::Relaxed);
    let first = task::spawn(spawn("ping", ping, first_on)).ok()?;
    wait_finished(2);
    wait_exited(&[first, second]);
    Some(ELAPSED_NS.load(Ordering::Relaxed) / (2 * u64::from(rounds.max(1))))
}

fn hog(_: usize) {
    while !STOP.load(Ordering::Relaxed) {
        core::hint::spin_loop();
    }
    finish();
}

/// Média e máximo, em ns, de quanto `sleep_until` atrasa em relação ao pedido.
pub fn wake_jitter_ns(samples: u32, loaded: bool) -> Option<(u64, u64)> {
    STOP.store(false, Ordering::Relaxed);
    FINISHED.store(0, Ordering::Relaxed);
    MAIN.store(task::current().as_u32() as usize, Ordering::Relaxed);
    let hog_id = if loaded {
        Some(
            task::spawn(Spawn {
                name: "hog",
                entry: hog,
                arg: 0,
                simd: false,
                params: SchedParams::fair(0),
                affinity: 1 << percpu::current().index(),
            })
            .ok()?,
        )
    } else {
        None
    };
    let (mut sum, mut max) = (0_u64, 0_u64);
    for sample in 0..samples {
        let wait = 500_000 + u64::from(sample) * 7_919 % 1_000 * 1_000;
        let deadline = platform::now_ns()? + wait;
        task::sleep_until(deadline);
        let late = platform::now_ns()?.saturating_sub(deadline);
        sum += late;
        max = max.max(late);
    }
    STOP.store(true, Ordering::Release);
    if let Some(hog_id) = hog_id {
        wait_finished(1);
        wait_exited(&[hog_id]);
    }
    Some((sum / u64::from(samples.max(1)), max))
}

/// Amostras de latência de acordar entre tarefas que o acordador guarda na pilha (a mediana as ordena).
const WAKE_TASK_MAX_SAMPLES: usize = 200;
/// Fatia curta do dorminhoco: menor que a do acordador, para `PREEMPT_SHORT` valer.
const SLEEPER_SLICE_NS: u64 = 100_000;
/// Acertos seguidos que encerram o aquecimento, e o teto de rodadas que ele pode gastar.
const WAKE_WARM_UP_HITS: u32 = 2;
const WAKE_WARM_UP_MAX: usize = 20;
/// Quanto o acordador espera o dorminhoco rodar antes de desistir da amostra.
const WAKE_TASK_GIVE_UP_NS: u64 = 5_000_000;

static LAT_RAN_AT: AtomicU64 = AtomicU64::new(0);
static LAT_SEQ: AtomicU32 = AtomicU32::new(0);
static LAT_SAMPLES: AtomicU32 = AtomicU32::new(0);
/// Mediana, média e máximo que o acordador calcula com as amostras na pilha dele.
static LAT_MEDIAN: AtomicU64 = AtomicU64::new(0);
static LAT_MEAN: AtomicU64 = AtomicU64::new(0);
static LAT_MAX: AtomicU64 = AtomicU64::new(0);
static LAT_SLEEPER: AtomicUsize = AtomicUsize::new(0);
/// Quantas amostras o dorminhoco já tinha rodado quando `unpark` voltou.
static LAT_DIRECT: AtomicU32 = AtomicU32::new(0);

/// Dorme em `park` e, ao acordar, carimba o relógio: o fim da latência de acordar.
fn latency_sleeper(_: usize) {
    while !STOP.load(Ordering::Acquire) {
        task::park();
        LAT_RAN_AT.store(platform::now_ns().unwrap_or(0), Ordering::Relaxed);
        LAT_SEQ.fetch_add(1, Ordering::Release);
    }
    finish();
}

/// Acorda o dorminhoco `LAT_SAMPLES` vezes, de uma tarefa da mesma CPU, e guarda o tempo entre
/// o `unpark` e ele rodar. Espera girando. As primeiras rodadas aquecem e não contam.
fn latency_waker(_: usize) {
    let sleeper = id(&LAT_SLEEPER);
    let samples = (LAT_SAMPLES.load(Ordering::Relaxed) as usize).min(WAKE_TASK_MAX_SAMPLES);
    let mut kept = [0_u64; WAKE_TASK_MAX_SAMPLES];
    let (mut warm, mut hits, mut counted) = (0, 0, 0);
    while counted < samples {
        while !task::is_blocked(sleeper) {
            task::yield_now();
        }
        let seq = LAT_SEQ.load(Ordering::Acquire);
        let start = platform::now_ns().unwrap_or(0);
        task::unpark(sleeper);
        let mut direct = LAT_SEQ.load(Ordering::Acquire) != seq;
        let mut late = WAKE_TASK_GIVE_UP_NS;
        while LAT_SEQ.load(Ordering::Acquire) == seq {
            if platform::now_ns().unwrap_or(u64::MAX) > start + WAKE_TASK_GIVE_UP_NS {
                direct = false;
                break;
            }
            core::hint::spin_loop();
        }
        if LAT_SEQ.load(Ordering::Acquire) != seq {
            late = LAT_RAN_AT.load(Ordering::Relaxed).saturating_sub(start);
        }
        if hits < WAKE_WARM_UP_HITS && warm < WAKE_WARM_UP_MAX {
            warm += 1;
            hits = if direct { hits + 1 } else { 0 };
            continue;
        }
        LAT_DIRECT.fetch_add(u32::from(direct), Ordering::Relaxed);
        kept[counted] = late;
        counted += 1;
    }
    let kept = &mut kept[..samples];
    insertion_sort(kept);
    LAT_MEDIAN.store(kept[samples / 2], Ordering::Relaxed);
    LAT_MEAN.store(
        kept.iter().sum::<u64>() / samples.max(1) as u64,
        Ordering::Relaxed,
    );
    LAT_MAX.store(kept[samples - 1], Ordering::Relaxed);
    finish();
}

/// Ordena até `WAKE_TASK_MAX_SAMPLES` valores (SDD 07, "Otimização da troca").
fn insertion_sort(values: &mut [u64]) {
    for next in 1..values.len() {
        let mut at = next;
        while at > 0 && values[at - 1] > values[at] {
            values.swap(at - 1, at);
            at -= 1;
        }
    }
}

/// Quanto custa acordar uma tarefa de fatia curta a partir de outra, na mesma CPU, em ns.
#[derive(Clone, Copy, Debug, Default)]
pub struct WakeLatency {
    pub mean_ns: u64,
    pub median_ns: u64,
    pub max_ns: u64,
    /// Quantas vezes o dorminhoco já tinha rodado quando `unpark` voltou.
    pub direct: u32,
    pub samples: u32,
}

impl WakeLatency {
    /// Em 90% das amostras o dorminhoco já tinha rodado quando `unpark` voltou.
    pub const fn passes(&self) -> bool {
        self.samples > 0 && self.direct * 10 >= self.samples * 9
    }
}

/// Latência de acordar entre tarefas na mesma CPU (SDD 15): o acordador cria um
/// dorminhoco de fatia curta e o acorda `samples` vezes. `None` se faltou slot.
pub fn wake_task_latency(samples: u32) -> Option<WakeLatency> {
    let count = usize::try_from(samples)
        .unwrap_or(WAKE_TASK_MAX_SAMPLES)
        .clamp(1, WAKE_TASK_MAX_SAMPLES);
    let samples = u32::try_from(count).unwrap_or(1);
    STOP.store(false, Ordering::Relaxed);
    FINISHED.store(0, Ordering::Relaxed);
    LAT_SEQ.store(0, Ordering::Relaxed);
    LAT_SAMPLES.store(samples, Ordering::Relaxed);
    LAT_DIRECT.store(0, Ordering::Relaxed);
    MAIN.store(task::current().as_u32() as usize, Ordering::Relaxed);
    let here = percpu::pinned(percpu::PerCpu::index);
    let spawn = |name, entry, params| Spawn {
        name,
        entry,
        arg: 0,
        simd: false,
        params,
        affinity: 1 << here,
    };
    let sleeper = task::spawn(spawn(
        "sleeper",
        latency_sleeper,
        SchedParams::fair_with_slice(0, SLEEPER_SLICE_NS),
    ))
    .ok()?;
    LAT_SLEEPER.store(sleeper.as_u32() as usize, Ordering::Relaxed);
    let waker = task::spawn(spawn("waker", latency_waker, SchedParams::fair(0))).ok()?;
    wait_finished(1);
    STOP.store(true, Ordering::Release);
    task::unpark(sleeper);
    wait_finished(2);
    wait_exited(&[waker, sleeper]);
    Some(WakeLatency {
        mean_ns: LAT_MEAN.load(Ordering::Relaxed),
        median_ns: LAT_MEDIAN.load(Ordering::Relaxed),
        max_ns: LAT_MAX.load(Ordering::Relaxed),
        direct: LAT_DIRECT.load(Ordering::Relaxed),
        samples,
    })
}

/// Autoteste: acordar uma tarefa de fatia curta troca na hora, sem esperar a IRQ. Imprime a
/// linha do boot e devolve se passou.
fn wake_self_test() -> bool {
    let Some(latency) = wake_task_latency(WAKE_SELF_TEST_SAMPLES) else {
        crate::console::print(format_args!(
            "[rustos] wake self-test: FAILED (no sample)\n"
        ));
        return false;
    };
    let verdict = if latency.passes() { "ok" } else { "FAILED" };
    crate::console::print(format_args!(
        "[rustos] wake self-test: {verdict} (task to task: mean {} ns, median {} ns, max {} ns, {} of {} direct)\n",
        latency.mean_ns, latency.median_ns, latency.max_ns, latency.direct, latency.samples
    ));
    latency.passes()
}

/// Interrupções do timer numa janela em que só a ociosa roda: sem tique periódico, só a
/// que acorda quem dorme (uma). Devolve a contagem e o tamanho da janela, em ms.
pub fn idle_timer_irqs() -> (u64, u64) {
    const WINDOW_MS: u64 = 50;
    let before = interrupts::ticks();
    let deadline = platform::now_ns().unwrap_or(0) + WINDOW_MS * 1_000_000;
    task::sleep_until(deadline);
    (interrupts::ticks() - before, WINDOW_MS)
}

/// Nanossegundos por `pick_next` e por `tick` da política, com 8 tarefas prontas e um
/// relógio de mentira que anda uma fatia (ou um décimo) por chamada: o custo é o do código.
pub fn policy_ns(rounds: u32) -> (u64, u64) {
    const TASKS: u32 = 8;
    let mut policy = Composite::<{ task::RT_SLOTS }, { task::MAX_TASKS }>::new();
    for raw in 0..TASKS {
        if policy
            .add(TaskId::new(raw), SchedParams::fair(0), 0)
            .is_err()
        {
            return (0, 0);
        }
    }
    let rounds = u64::from(rounds.max(1));
    let (mut pick, mut tick) = (u64::MAX, u64::MAX);
    let mut now = 0_u64;
    for _ in 0..4 {
        let start = platform::cycles();
        for _ in 0..rounds {
            now += 700_000;
            core::hint::black_box(policy.pick_next(now));
        }
        let spent = platform::cycles_to_ns(platform::cycles().wrapping_sub(start));
        pick = pick.min(spent / rounds);
        let start = platform::cycles();
        for _ in 0..rounds {
            now += 70_000;
            core::hint::black_box(policy.tick(now));
        }
        let spent = platform::cycles_to_ns(platform::cycles().wrapping_sub(start));
        tick = tick.min(spent / rounds);
    }
    (pick, tick)
}

/// Gancho do IPI de reescalonar: a medida de ida e volta o usa.
pub fn on_ipi() {
    if !ECHO_ON.load(Ordering::Acquire) {
        return;
    }
    let me = percpu::current().index();
    if me == ECHO_FAR.load(Ordering::Relaxed) {
        let origin = percpu::get(ECHO_NEAR.load(Ordering::Relaxed)).hw_id();
        // SAFETY: a medida só roda com outra CPU ativa, então a interface está ligada.
        unsafe { platform::ipi() }.send(origin, IpiKind::Reschedule);
    } else if me == ECHO_NEAR.load(Ordering::Relaxed) {
        ECHO_BACK.fetch_add(1, Ordering::Release);
    }
}

static ECHO_ON: AtomicBool = AtomicBool::new(false);
static ECHO_NEAR: AtomicUsize = AtomicUsize::new(0);
static ECHO_FAR: AtomicUsize = AtomicUsize::new(0);
static ECHO_BACK: AtomicU32 = AtomicU32::new(0);

static ECHO_ROUNDS: AtomicU32 = AtomicU32::new(0);

/// Roda presa à CPU `near`: manda o IPI à `far`, que o devolve do próprio handler (`on_ipi`),
/// e conta o tempo de `ECHO_ROUNDS` idas e voltas. Melhor de três.
fn ipi_probe(_: usize) {
    let rounds = ECHO_ROUNDS.load(Ordering::Relaxed);
    let target = percpu::get(ECHO_FAR.load(Ordering::Relaxed)).hw_id();
    // SAFETY: há outra CPU ativa, então a interface de interrupção está ligada.
    let ipi = unsafe { platform::ipi() };
    let mut best = u64::MAX;
    for _ in 0..4 {
        let first = ECHO_BACK.load(Ordering::Acquire);
        let start = platform::cycles();
        for round in 1..=rounds {
            ipi.send(target, IpiKind::Reschedule);
            while ECHO_BACK.load(Ordering::Acquire) < first + round {
                core::hint::spin_loop();
            }
        }
        let spent = platform::cycles_to_ns(platform::cycles().wrapping_sub(start));
        best = best.min(spent / u64::from(rounds.max(1)));
    }
    ELAPSED_NS.store(best, Ordering::Relaxed);
    finish();
}

/// IPI de ida e volta entre a CPU de quem mede e outra. `None` com uma CPU só.
pub fn ipi_round_trip_ns(rounds: u32) -> Option<u64> {
    let (near, far) = cpu_pair();
    let far = far?;
    ECHO_NEAR.store(near, Ordering::Relaxed);
    ECHO_FAR.store(far, Ordering::Relaxed);
    ECHO_BACK.store(0, Ordering::Relaxed);
    ECHO_ROUNDS.store(rounds, Ordering::Relaxed);
    FINISHED.store(0, Ordering::Relaxed);
    MAIN.store(task::current().as_u32() as usize, Ordering::Relaxed);
    ECHO_ON.store(true, Ordering::Release);
    let probe = task::spawn(Spawn {
        name: "ipi",
        entry: ipi_probe,
        arg: 0,
        simd: false,
        params: SchedParams::fair(0),
        affinity: 1 << near,
    })
    .ok()?;
    wait_finished(1);
    ECHO_ON.store(false, Ordering::Release);
    wait_exited(&[probe]);
    Some(ELAPSED_NS.load(Ordering::Relaxed))
}

static WORK_ITERS: AtomicU64 = AtomicU64::new(0);

/// Um trabalho só de CPU, sem memória nem sincronização: `iterations` voltas.
fn burn(iterations: u64) -> u64 {
    let mut acc = 0x9E37_79B9_7F4A_7C15_u64;
    for index in 0..iterations {
        acc = core::hint::black_box(acc.rotate_left(5) ^ index).wrapping_mul(0x2545_F491_4F6C_DD1D);
    }
    acc
}

fn worker(_: usize) {
    core::hint::black_box(burn(WORK_ITERS.load(Ordering::Relaxed)));
    finish();
}

/// Voltas de `burn` por milissegundo, medidas agora: o trabalho de cada tarefa da escala é
/// escolhido em tempo, não em voltas, para valer no TCG e no HVF.
fn burn_per_ms() -> u64 {
    let probe = 50_000;
    let start = platform::cycles();
    core::hint::black_box(burn(probe));
    let ns = platform::cycles_to_ns(platform::cycles().wrapping_sub(start)).max(1);
    probe * 1_000_000 / ns
}

/// Tempo total, em ns, de `tasks` tarefas de CPU de `task_ms` ms cada, presas às `cpus`
/// primeiras CPUs ativas: dá a escala do escalonador com o número de núcleos.
pub fn scale_ns(cpus: usize, tasks: usize, task_ms: u64) -> Option<u64> {
    let active: Vec<usize> = (0..rustos_hal::MAX_CPUS)
        .filter(|&index| task::active_cpus() >> index & 1 != 0)
        .collect();
    if active.len() < cpus {
        return None;
    }
    let mask = active
        .iter()
        .take(cpus)
        .fold(0_u32, |mask, &index| mask | 1 << index);
    WORK_ITERS.store(burn_per_ms() * task_ms, Ordering::Relaxed);
    FINISHED.store(0, Ordering::Relaxed);
    MAIN.store(task::current().as_u32() as usize, Ordering::Relaxed);
    let start = platform::now_ns()?;
    let mut ids = Vec::new();
    for _ in 0..tasks {
        ids.push(
            task::spawn(Spawn {
                name: "work",
                entry: worker,
                arg: 0,
                simd: false,
                params: SchedParams::fair(0),
                affinity: mask,
            })
            .ok()?,
        );
    }
    wait_finished(u32::try_from(tasks).ok()?);
    let elapsed = platform::now_ns()?.saturating_sub(start);
    wait_exited(&ids);
    Some(elapsed)
}

static STORM_HEAP: AtomicBool = AtomicBool::new(false);
static STORM_ITERS: AtomicU32 = AtomicU32::new(0);

/// Cada CPU aloca e devolve sem parar: frames soltos (4 por volta) ou objetos do heap.
fn storm_worker(_: usize) {
    let iterations = STORM_ITERS.load(Ordering::Relaxed);
    if STORM_HEAP.load(Ordering::Relaxed) {
        for round in 0..iterations {
            let object = alloc::boxed::Box::new([round.to_le_bytes()[0]; 64]);
            core::hint::black_box(&object);
        }
    } else if let Some(mut frames) = crate::frames::handle() {
        for _ in 0..iterations {
            let mut held = [None; 4];
            for slot in &mut held {
                *slot = rustos_hal::FrameSource::allocate_frame(&mut frames);
            }
            for frame in held.into_iter().flatten() {
                crate::frames::free_frame(frames, frame);
            }
        }
    }
    finish();
}

/// Tempo total e disputas de `cpus` tarefas, uma por CPU, alocando em paralelo. `heap` escolhe
/// o heap (um lock global) em vez dos frames soltos.
fn storm_ns(cpus: usize, iterations: u32, heap: bool) -> Option<(u64, u32)> {
    let active: Vec<usize> = (0..rustos_hal::MAX_CPUS)
        .filter(|&index| task::active_cpus() >> index & 1 != 0)
        .collect();
    if active.len() < cpus {
        return None;
    }
    STORM_HEAP.store(heap, Ordering::Relaxed);
    STORM_ITERS.store(iterations, Ordering::Relaxed);
    FINISHED.store(0, Ordering::Relaxed);
    MAIN.store(task::current().as_u32() as usize, Ordering::Relaxed);
    let before = if heap {
        crate::heap::contentions()
    } else {
        crate::frames::contentions()
    };
    let start = platform::now_ns()?;
    let mut ids = Vec::new();
    for &cpu in active.iter().take(cpus) {
        ids.push(
            task::spawn(Spawn {
                name: "storm",
                entry: storm_worker,
                arg: 0,
                simd: false,
                params: SchedParams::fair(0),
                affinity: 1 << cpu,
            })
            .ok()?,
        );
    }
    wait_finished(u32::try_from(cpus).ok()?);
    let elapsed = platform::now_ns()?.saturating_sub(start);
    wait_exited(&ids);
    let after = if heap {
        crate::heap::contentions()
    } else {
        crate::frames::contentions()
    };
    Some((elapsed, after - before))
}

/// A mesma carga de frames com o cache por CPU desligado e ligado, e a do heap: o número
/// que decide se o cache vale (SDD 12).
pub fn memory_storm(iterations: u32) -> StormSample {
    let cpus = task::active_cpus().count_ones() as usize;
    crate::frames::set_cache(false);
    let locked = storm_ns(cpus, iterations, false);
    crate::frames::set_cache(true);
    let cached = storm_ns(cpus, iterations, false);
    let heap = storm_ns(cpus, iterations, true);
    StormSample {
        cpus: u8::try_from(cpus).unwrap_or(0),
        iterations,
        locked_ns: locked.map_or(0, |(ns, _)| ns),
        locked_contention: locked.map_or(0, |(_, count)| count),
        cached_ns: cached.map_or(0, |(ns, _)| ns),
        cached_contention: cached.map_or(0, |(_, count)| count),
        heap_ns: heap.map_or(0, |(ns, _)| ns),
        heap_contention: heap.map_or(0, |(_, count)| count),
    }
}

/// Gira a CPU por `ms` antes da primeira medida (SDD 15, "Medidas").
fn warm_up(ms: u64) {
    let end = platform::now_ns().unwrap_or(0) + ms * 1_000_000;
    while platform::now_ns().unwrap_or(u64::MAX) < end {
        core::hint::black_box(burn(1_000));
    }
}

pub fn run(effort: Effort) -> Option<BenchReport> {
    warm_up(effort.warmup_ms);
    let plain = context_switch_ns(effort.rounds, false, Placement::SameCpu)?;
    let simd = context_switch_ns(effort.rounds, true, Placement::SameCpu)?;
    let cross =
        context_switch_ns(effort.rounds / CROSS_DIVISOR, false, Placement::CrossCpu).unwrap_or(0);
    let ipi_rtt_ns = ipi_round_trip_ns(effort.rounds / CROSS_DIVISOR).unwrap_or(0);
    let (idle_mean, idle_max) = wake_jitter_ns(effort.samples, false)?;
    let (load_mean, load_max) = wake_jitter_ns(effort.samples, true)?;
    let wake_task = wake_task_latency(effort.samples).unwrap_or_default();
    let (pick_next_ns, tick_ns) = policy_ns(effort.rounds);
    let mut scale = [ScaleSample::default(); 3];
    for (sample, cpus) in scale.iter_mut().zip([1_usize, 2, 4]) {
        if let Some(ns) = scale_ns(cpus, SCALE_TASKS, effort.task_ms) {
            *sample = ScaleSample {
                cpus: u8::try_from(cpus).unwrap_or(0),
                tasks: u8::try_from(SCALE_TASKS).unwrap_or(0),
                total_ns: ns,
            };
        }
    }
    let storm = memory_storm(effort.storm);
    Some(BenchReport {
        storm,
        switch_ns: plain,
        switch_simd_ns: simd,
        switch_cross_ns: cross,
        ipi_rtt_ns,
        scale,
        cpus: u8::try_from(task::active_cpus().count_ones()).unwrap_or(0),
        wake_samples: effort.samples,
        wake_mean_ns: idle_mean,
        wake_max_ns: idle_max,
        wake_loaded_mean_ns: load_mean,
        wake_loaded_max_ns: load_max,
        wake_task_median_ns: wake_task.median_ns,
        wake_task_mean_ns: wake_task.mean_ns,
        wake_task_max_ns: wake_task.max_ns,
        pick_next_ns,
        tick_ns,
    })
}

static SIMD_FAILURES: AtomicU32 = AtomicU32::new(0);
static SIMD_DONE: AtomicU32 = AtomicU32::new(0);

/// Cada trabalhador põe um valor próprio em `xmm0`/`v0` e o confere sem parar
/// por 40 ms. O 0 nunca cede a CPU: só a preempção do timer o interrompe.
fn simd_worker(index: usize) {
    let mut pattern = [0_u8; 16];
    for (offset, byte) in pattern.iter_mut().enumerate() {
        *byte = u8::try_from(index * 16 + offset + 1).unwrap_or(0xFF);
    }
    platform::fpu_write_probe(&pattern);
    let end = platform::now_ns().unwrap_or(0) + 40_000_000;
    let mut turn = 0_u32;
    while platform::now_ns().unwrap_or(u64::MAX) < end {
        if platform::fpu_read_probe() != pattern {
            SIMD_FAILURES.fetch_add(1, Ordering::Relaxed);
            break;
        }
        turn += 1;
        if index != 0 && turn.is_multiple_of(256) {
            task::yield_now();
        }
    }
    SIMD_DONE.fetch_add(1, Ordering::Release);
    task::unpark(id(&MAIN));
}

/// Dois trabalhadores de SIMD, cada um com o seu valor nos registradores. Devolve quantas
/// vezes o primeiro foi escolhido para rodar, ou `None` se algum viu o valor trocar.
fn simd_workers(affinity: u32) -> Option<u64> {
    const WORKERS: u32 = 2;
    SIMD_FAILURES.store(0, Ordering::Relaxed);
    SIMD_DONE.store(0, Ordering::Relaxed);
    MAIN.store(task::current().as_u32() as usize, Ordering::Relaxed);
    let mut first = None;
    for index in 0..WORKERS as usize {
        let spawned = task::spawn(Spawn {
            name: "simd",
            entry: simd_worker,
            arg: index,
            simd: true,
            params: SchedParams::fair(0),
            affinity,
        })
        .ok()?;
        first.get_or_insert(spawned);
    }
    while SIMD_DONE.load(Ordering::Acquire) < WORKERS {
        task::park();
    }
    let first = first?;
    (SIMD_FAILURES.load(Ordering::Relaxed) == 0).then(|| task::switches(first))
}

/// O estado de SIMD de cada tarefa sobrevive à troca: dois trabalhadores na mesma CPU
/// alternam por preempção e por `yield`, e depois dois soltos pelas CPUs (migram, se o
/// balanceamento quiser). Devolve quantas vezes o primeiro rodou na CPU compartilhada.
pub fn simd_self_test() -> Option<u64> {
    let here = 1 << platform_cpu_index();
    let preempted = simd_workers(here)?;
    simd_workers(0)?;
    (preempted >= 3).then_some(preempted)
}

fn platform_cpu_index() -> usize {
    crate::percpu::current().index()
}

const CHURN_TASKS: usize = 8;
static CHURN_RUNNING: [AtomicBool; CHURN_TASKS] = [const { AtomicBool::new(false) }; CHURN_TASKS];
static CHURN_CPU: [AtomicUsize; CHURN_TASKS] =
    [const { AtomicUsize::new(usize::MAX) }; CHURN_TASKS];
static CHURN_DONE: AtomicU32 = AtomicU32::new(0);
static CHURN_DOUBLE_RUNS: AtomicU32 = AtomicU32::new(0);
static CHURN_MIGRATIONS: AtomicU64 = AtomicU64::new(0);
static CHURN_TURNS: AtomicU64 = AtomicU64::new(0);

/// Roda, cede, dorme e acorda por 40 ms. A bandeira de `CHURN_RUNNING` sobe ao começar cada
/// volta e desce ao fim: se duas CPUs rodassem a mesma tarefa ao mesmo tempo, a segunda a veria subida.
fn churn(index: usize) {
    let end = platform::now_ns().unwrap_or(0) + 40_000_000;
    let mut turn = 0_u32;
    while platform::now_ns().unwrap_or(u64::MAX) < end {
        if CHURN_RUNNING[index].swap(true, Ordering::AcqRel) {
            CHURN_DOUBLE_RUNS.fetch_add(1, Ordering::Relaxed);
        }
        let here = percpu::current().index();
        let before = CHURN_CPU[index].swap(here, Ordering::Relaxed);
        if before != usize::MAX && before != here {
            CHURN_MIGRATIONS.fetch_add(1, Ordering::Relaxed);
        }
        core::hint::black_box(burn(2_000));
        CHURN_RUNNING[index].store(false, Ordering::Release);
        CHURN_TURNS.fetch_add(1, Ordering::Relaxed);
        match turn % 3 {
            0 => task::yield_now(),
            1 => task::sleep_until(platform::now_ns().unwrap_or(0) + 100_000),
            _ => {}
        }
        turn += 1;
    }
    CHURN_DONE.fetch_add(1, Ordering::Release);
    task::unpark(id(&MAIN));
}

static HOG_STOP: AtomicBool = AtomicBool::new(false);
static HOG_TURNS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

fn counting_hog(index: usize) {
    while !HOG_STOP.load(Ordering::Relaxed) {
        HOG_TURNS[index].fetch_add(1, Ordering::Relaxed);
    }
}

/// Duas tarefas de CPU presas à CPU de quem mede, e quem mede gira 100 ms sem ceder: só a
/// preempção do timer dá a CPU às duas. Se o prazo da fatia não foi armado ao pôr a tarefa na
/// fila local (ninguém cede nem há IRQ), as duas ficam com zero voltas.
fn preempt_by_timer() -> Option<(u64, u64)> {
    HOG_STOP.store(false, Ordering::Relaxed);
    let here = percpu::pinned(percpu::PerCpu::index);
    let mut ids = Vec::new();
    for (index, turns) in HOG_TURNS.iter().enumerate() {
        turns.store(0, Ordering::Relaxed);
        ids.push(
            task::spawn(Spawn {
                name: "hog",
                entry: counting_hog,
                arg: index,
                simd: false,
                params: SchedParams::fair(0),
                affinity: 1 << here,
            })
            .ok()?,
        );
    }
    let end = platform::now_ns()? + 100_000_000;
    while platform::now_ns()? < end {
        core::hint::spin_loop();
    }
    HOG_STOP.store(true, Ordering::Release);
    wait_exited(&ids);
    let turns = (
        HOG_TURNS[0].load(Ordering::Relaxed),
        HOG_TURNS[1].load(Ordering::Relaxed),
    );
    (turns.0 > 0 && turns.1 > 0).then_some(turns)
}

/// Tarefas que rodam, cedem e dormem nas CPUs todas, e migram quando o balanceamento ou o
/// acordar quer: nenhuma roda em duas CPUs ao mesmo tempo e nenhuma se perde. Devolve as
/// migrações e as voltas, ou `None` se algo falhou.
pub fn smp_self_test() -> Option<(u64, u64)> {
    CHURN_DONE.store(0, Ordering::Relaxed);
    CHURN_DOUBLE_RUNS.store(0, Ordering::Relaxed);
    CHURN_MIGRATIONS.store(0, Ordering::Relaxed);
    CHURN_TURNS.store(0, Ordering::Relaxed);
    MAIN.store(task::current().as_u32() as usize, Ordering::Relaxed);
    let mut ids = Vec::new();
    for index in 0..CHURN_TASKS {
        CHURN_RUNNING[index].store(false, Ordering::Relaxed);
        CHURN_CPU[index].store(usize::MAX, Ordering::Relaxed);
        ids.push(
            task::spawn(Spawn {
                name: "churn",
                entry: churn,
                arg: index,
                simd: false,
                params: SchedParams::fair(0),
                affinity: 0,
            })
            .ok()?,
        );
    }
    let deadline = platform::now_ns()? + 2_000_000_000;
    while CHURN_DONE.load(Ordering::Acquire) < u32::try_from(CHURN_TASKS).ok()? {
        if platform::now_ns()? > deadline {
            return None;
        }
        task::sleep_until(platform::now_ns()? + 1_000_000);
    }
    wait_exited(&ids);
    preempt_by_timer()?;
    wake_self_test().then_some(())?;
    (CHURN_DOUBLE_RUNS.load(Ordering::Relaxed) == 0).then(|| {
        (
            CHURN_MIGRATIONS.load(Ordering::Relaxed),
            CHURN_TURNS.load(Ordering::Relaxed),
        )
    })
}

fn panicker(_: usize) {
    panic!("teste de pânico na CPU {}", percpu::current().index());
}

/// Cria uma tarefa presa a outra CPU que entra em pânico. Só volta (`false`) se não há outra
/// CPU ativa; depois do pânico o sistema inteiro para.
pub fn panic_on_other_cpu() -> bool {
    let (_, other) = cpu_pair();
    let Some(other) = other else {
        return false;
    };
    let spawned = task::spawn(Spawn {
        name: "panicker",
        entry: panicker,
        arg: 0,
        simd: false,
        params: SchedParams::fair(0),
        affinity: 1 << other,
    });
    if spawned.is_err() {
        return false;
    }
    loop {
        task::sleep_until(platform::now_ns().unwrap_or(0) + 10_000_000);
    }
}
