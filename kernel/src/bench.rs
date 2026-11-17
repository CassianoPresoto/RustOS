//! Medidas e autoteste das tarefas: latência de troca de contexto (o equivalente
//! do `lat_ctx` do lmbench), jitter de acordar e o estado de SIMD por tarefa.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use rustos_sched::{Composite, SchedParams, Scheduler, TaskId};
use rustos_shell::BenchReport;

use crate::task::{self, Spawn};
use crate::{interrupts, platform};

/// Quanto cada medida repete.
#[derive(Clone, Copy, Debug)]
pub struct Effort {
    pub rounds: u32,
    pub samples: u32,
}

/// O curto do boot e o completo do comando `bench`.
pub const BOOT: Effort = Effort {
    rounds: 2_000,
    samples: 20,
};
pub const FULL: Effort = Effort {
    rounds: 20_000,
    samples: 200,
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

/// Nanossegundos por troca de contexto: um aquecimento e o melhor de três rodadas.
pub fn context_switch_ns(rounds: u32, simd: bool) -> Option<u64> {
    ping_pong(rounds, simd)?;
    let mut best = u64::MAX;
    for _ in 0..3 {
        best = best.min(ping_pong(rounds, simd)?);
    }
    Some(best)
}

/// Duas tarefas passam a vez uma à outra por `unpark` e `park`; cada ida e volta
/// são duas trocas.
fn ping_pong(rounds: u32, simd: bool) -> Option<u64> {
    STOP.store(false, Ordering::Relaxed);
    FINISHED.store(0, Ordering::Relaxed);
    ROUNDS.store(rounds, Ordering::Relaxed);
    MAIN.store(task::current().as_u32() as usize, Ordering::Relaxed);
    let spawn = |name, entry| Spawn {
        name,
        entry,
        arg: 0,
        simd,
        params: SchedParams::fair(0),
    };
    let second = task::spawn(spawn("pong", pong)).ok()?;
    PONG.store(second.as_u32() as usize, Ordering::Relaxed);
    let first = task::spawn(spawn("ping", ping)).ok()?;
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

pub fn run(effort: Effort) -> Option<BenchReport> {
    let plain = context_switch_ns(effort.rounds, false)?;
    let simd = context_switch_ns(effort.rounds, true)?;
    let (idle_mean, idle_max) = wake_jitter_ns(effort.samples, false)?;
    let (load_mean, load_max) = wake_jitter_ns(effort.samples, true)?;
    let (pick_next_ns, tick_ns) = policy_ns(effort.rounds);
    Some(BenchReport {
        switch_ns: plain,
        switch_simd_ns: simd,
        wake_samples: effort.samples,
        wake_mean_ns: idle_mean,
        wake_max_ns: idle_max,
        wake_loaded_mean_ns: load_mean,
        wake_loaded_max_ns: load_max,
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

/// Duas tarefas de SIMD alternam por preempção e por `yield`, cada uma com o seu
/// valor nos registradores. Devolve quantas vezes a primeira foi escolhida para rodar.
pub fn simd_self_test() -> Option<u64> {
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
        })
        .ok()?;
        first.get_or_insert(spawned);
    }
    while SIMD_DONE.load(Ordering::Acquire) < WORKERS {
        task::park();
    }
    let preempted = task::switches(first?);
    (SIMD_FAILURES.load(Ordering::Relaxed) == 0 && preempted >= 3).then_some(preempted)
}
