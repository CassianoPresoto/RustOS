//! Medição no host, só com `--ignored`: `cargo test --release -p rustos-sched bench --
//! --ignored --nocapture --test-threads=1`. Não é QEMU nem metal: compara as políticas
//! entre si.

use core::hint::black_box;
use std::time::Instant;

use crate::TaskId;
use crate::eevdf::Eevdf;
use crate::round_robin::RoundRobin;
use crate::scheduler::{SchedParams, Scheduler};
use crate::sim::ratio;

const ROUNDS: usize = 7;
const ITERS: u64 = 100_000;
const STEP: u64 = 10_000_000;

fn id(raw: usize) -> TaskId {
    TaskId::new(u32::try_from(raw).unwrap())
}

fn best_ns(mut body: impl FnMut()) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..ROUNDS {
        let start = Instant::now();
        for _ in 0..ITERS {
            body();
        }
        let nanos = u64::try_from(start.elapsed().as_nanos()).unwrap();
        best = best.min(ratio(nanos, ITERS));
    }
    best
}

fn populate<S: Scheduler>(sched: &mut S, tasks: usize) -> u64 {
    for raw in 0..tasks {
        let nice = i8::try_from(raw % 7).unwrap() - 3;
        sched.add(id(raw), SchedParams::fair(nice), 0).unwrap();
    }
    let mut now = 0;
    for _ in 0..tasks * 20 {
        now += STEP;
        sched.tick(now);
        sched.pick_next(now);
    }
    now
}

struct Numbers {
    tick: f64,
    tick_expiring: f64,
    pick: f64,
    wake_sleep: f64,
}

fn measure<S: Scheduler>(sched: &mut S, tasks: usize) -> Numbers {
    let mut now = populate(sched, tasks);
    let tick = best_ns(|| {
        now += 100;
        black_box(sched.tick(black_box(now)));
    });
    let tick_expiring = best_ns(|| {
        now += STEP;
        black_box(sched.tick(black_box(now)));
    });
    let cycle = best_ns(|| {
        now += STEP;
        sched.tick(now);
        black_box(sched.pick_next(black_box(now)));
    });
    let mut victim = 0;
    let wake_sleep = best_ns(|| {
        victim = (victim + 1) % tasks;
        let task = id(victim);
        if sched.current() != Some(task) {
            sched.sleep(task, now);
            sched.wake(task, now);
        }
    });
    Numbers {
        tick,
        tick_expiring,
        pick: (cycle - tick_expiring).max(0.0),
        wake_sleep,
    }
}

#[test]
#[ignore = "benchmark: rode com --ignored"]
fn bench_round_robin_against_eevdf() {
    eprintln!("ns por chamada; pick_next = (tick expirando + pick_next) - tick expirando");
    eprintln!("tarefas | política | tick | tick expirando | pick_next | sleep+wake");
    for tasks in [8, 64, 512] {
        let rr = measure(&mut *Box::new(RoundRobin::<512>::new()), tasks);
        let eevdf = measure(&mut *Box::new(Eevdf::<512>::new()), tasks);
        for (name, n) in [("rodízio", rr), ("eevdf", eevdf)] {
            eprintln!(
                "{tasks:>7} | {name:<8} | {:>5.1} | {:>14.1} | {:>9.1} | {:>10.1}",
                n.tick, n.tick_expiring, n.pick, n.wake_sleep
            );
        }
    }
}

fn tree_against_scan<const N: usize>() {
    let mut sched = Box::new(Eevdf::<N>::new());
    populate(&mut *sched, N);
    let tree = best_ns(|| {
        black_box(sched.pick_for_bench(black_box(false)));
    });
    let scan = best_ns(|| {
        black_box(sched.pick_for_bench(black_box(true)));
    });
    eprintln!("{N:>7} | {tree:>6.1} | {scan:>9.1}");
}

#[test]
#[ignore = "benchmark: rode com --ignored"]
fn bench_tree_against_linear_scan_for_the_eevdf_pick() {
    eprintln!("ns por escolha do elegível de menor prazo, capacidade igual às tarefas");
    eprintln!("tarefas | árvore | varredura");
    tree_against_scan::<8>();
    tree_against_scan::<64>();
    tree_against_scan::<512>();
}

#[test]
#[ignore = "benchmark: rode com --ignored"]
fn bench_memory_per_task() {
    eprintln!(
        "bytes: eevdf<512> {} ({} por tarefa), rodízio<512> {} ({} por tarefa), fifo<512> {}",
        size_of::<Eevdf<512>>(),
        size_of::<Eevdf<512>>() / 512,
        size_of::<RoundRobin<512>>(),
        size_of::<RoundRobin<512>>() / 512,
        size_of::<crate::Fifo<512>>(),
    );
    eprintln!(
        "bytes: Entity {}, nó da árvore {}",
        size_of::<crate::eevdf::Entity>(),
        crate::tree::NODE_SIZE
    );
}
