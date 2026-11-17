//! Cenários de ponta a ponta: justiça, latência e tempo real, medidos na simulação.

use crate::TaskId;
use crate::composite::{Composite, RtBudget};
use crate::eevdf::{Eevdf, EevdfConfig};
use crate::round_robin::RoundRobin;
use crate::scheduler::{SchedParams, Scheduler};
use crate::sim::{Kind, SimTask, Timer, run};

const SECOND: u64 = 1_000_000_000;

fn hog(params: SchedParams) -> SimTask {
    SimTask {
        params,
        kind: Kind::Hog,
    }
}

fn burst(params: SchedParams, run_ns: u64, sleep_ns: u64) -> SimTask {
    SimTask {
        params,
        kind: Kind::Burst { run_ns, sleep_ns },
    }
}

fn weights(nices: &[i8]) -> Vec<f64> {
    nices
        .iter()
        .map(|nice| f64::from(crate::weight::weight_of_nice(*nice)))
        .collect()
}

fn assert_proportional(nices: &[i8], timer: Timer, tolerance: f64) {
    let tasks: Vec<_> = nices.iter().map(|n| hog(SchedParams::fair(*n))).collect();
    let report = run(&mut Eevdf::<16>::new(), &tasks, 4 * SECOND, timer);
    let weights = weights(nices);
    let total: f64 = weights.iter().sum();
    assert_eq!(report.idle_ns, 0);
    for (i, weight) in weights.iter().enumerate() {
        let share = report.share(i);
        let expected = weight / total;
        assert!(
            (share - expected).abs() <= tolerance,
            "{timer:?} nice {nices:?}: tarefa {i} teve {share:.4}, esperado {expected:.4}"
        );
    }
}

#[test]
fn cpu_share_follows_the_weights_with_a_periodic_tick() {
    assert_proportional(&[0, 0], Timer::Periodic(1_000_000), 0.002);
    assert_proportional(&[0, 5, -5, 10], Timer::Periodic(1_000_000), 0.003);
    assert_proportional(&[-20, 0, 19], Timer::Periodic(250_000), 0.003);
    assert_proportional(&[0, 1, 2, 3, 4, 5, 6, 7], Timer::Periodic(1_000_000), 0.003);
}

#[test]
fn cpu_share_follows_the_weights_with_a_one_shot_timer() {
    assert_proportional(&[0, 5, -5, 10], Timer::OneShot, 0.001);
    assert_proportional(&[-10, 10], Timer::OneShot, 0.001);
}

#[test]
fn one_shot_timer_needs_far_fewer_interrupts_when_alone() {
    let report = run(
        &mut Eevdf::<4>::new(),
        &[hog(SchedParams::fair(0))],
        SECOND,
        Timer::OneShot,
    );
    assert_eq!(report.switches, 1);
}

fn interactive_vs_batch<S: Scheduler>(
    mut sched: S,
    slice_aware: bool,
    timer: Timer,
) -> (f64, f64, u64, u64) {
    let (interactive, batch) = if slice_aware {
        (
            SchedParams::fair_with_slice(0, 100_000),
            SchedParams::fair_with_slice(0, 6_000_000),
        )
    } else {
        (SchedParams::fair(0), SchedParams::fair(0))
    };
    let mut tasks = vec![hog(SchedParams::fair(0)); 4];
    tasks.push(burst(interactive, 200_000, 5_000_000));
    tasks.push(burst(batch, 200_000, 7_000_000));
    let report = run(&mut sched, &tasks, 10 * SECOND, timer);
    (
        report.mean_latency_ns(4),
        report.mean_latency_ns(5),
        report.max_latency_ns(4),
        report.max_latency_ns(5),
    )
}

#[test]
fn a_short_slice_interactive_task_runs_sooner_after_waking_than_a_batch_one() {
    for timer in [Timer::OneShot, Timer::Periodic(1_000_000)] {
        let (interactive, batch, interactive_max, batch_max) =
            interactive_vs_batch(Eevdf::<16>::new(), true, timer);
        eprintln!(
            "eevdf {timer:?}: interativa média {interactive:.0} ns, máx {interactive_max} ns; lote média {batch:.0} ns, máx {batch_max} ns"
        );
        assert!(
            interactive * 4.0 < batch,
            "{timer:?}: {interactive} contra {batch}"
        );
        assert!(interactive_max < batch_max);
    }
}

#[test]
fn the_interactive_wait_is_a_few_slices_at_worst_and_nearly_zero_on_average() {
    let (mean, _, max, _) = interactive_vs_batch(Eevdf::<16>::new(), true, Timer::OneShot);
    assert!(mean < 50_000.0, "{mean}");
    assert!(max <= 4 * crate::eevdf::BASE_SLICE_NS, "{max}");
}

#[test]
fn with_equal_slices_eevdf_still_beats_round_robin_for_the_sleeper() {
    let timer = Timer::Periodic(1_000_000);
    let (eevdf, _, _, _) = interactive_vs_batch(Eevdf::<16>::new(), false, timer);
    let (rr, _, _, _) = interactive_vs_batch(RoundRobin::<16>::new(), false, timer);
    eprintln!("fatias iguais: eevdf {eevdf:.0} ns, rodízio {rr:.0} ns");
    assert!(eevdf < rr, "{eevdf} contra {rr}");
}

#[test]
fn a_sleeper_does_not_starve_the_hogs_nor_get_more_than_its_demand() {
    let tasks = [
        hog(SchedParams::fair(0)),
        hog(SchedParams::fair(0)),
        burst(SchedParams::fair_with_slice(0, 100_000), 200_000, 5_000_000),
    ];
    let report = run(&mut Eevdf::<8>::new(), &tasks, 10 * SECOND, Timer::OneShot);
    let demand = 200_000.0 / 5_200_000.0;
    assert!(
        (report.share(2) - demand).abs() < 0.005,
        "{}",
        report.share(2)
    );
    assert!((report.share(0) - report.share(1)).abs() < 0.003);
}

type Rt = Composite<8, 16>;

#[test]
fn a_fifo_wakeup_preempts_the_fair_class_at_once() {
    let mut sched = Rt::new();
    sched.add(TaskId::new(1), SchedParams::fair(0), 0).unwrap();
    assert_eq!(sched.pick_next(0), Some(TaskId::new(1)));
    sched
        .add(TaskId::new(2), SchedParams::fifo(10), 1_000)
        .unwrap();
    assert!(sched.needs_resched());
    assert_eq!(sched.pick_next(1_000), Some(TaskId::new(2)));
    sched.sleep(TaskId::new(2), 2_000);
    assert_eq!(sched.pick_next(2_000), Some(TaskId::new(1)));
    sched.wake(TaskId::new(2), 3_000);
    assert!(sched.needs_resched());
    assert_eq!(sched.pick_next(3_000), Some(TaskId::new(2)));
}

#[test]
fn fifo_latency_is_zero_next_to_hogs() {
    let mut tasks = vec![hog(SchedParams::fair(0)); 4];
    tasks.push(burst(SchedParams::fifo(50), 100_000, 3_000_000));
    let report = run(
        &mut Rt::new(),
        &tasks,
        5 * SECOND,
        Timer::Periodic(1_000_000),
    );
    assert_eq!(report.max_latency_ns(4), 0);
}

#[test]
fn the_rt_cap_leaves_the_fair_class_five_percent_per_window() {
    let tasks = [hog(SchedParams::fifo(50)), hog(SchedParams::fair(0))];
    for timer in [Timer::OneShot, Timer::Periodic(1_000_000)] {
        let report = run(&mut Rt::new(), &tasks, 10 * SECOND, timer);
        let (rt, fair) = (report.share(0), report.share(1));
        eprintln!("{timer:?}: tempo real {rt:.4}, justa {fair:.4}");
        assert!(rt <= 0.9505, "{timer:?}: teto furado, {rt}");
        assert!(rt >= 0.945, "{timer:?}: o teto não pode sobrar, {rt}");
        assert!(fair >= 0.0495, "{timer:?}: a justa passou fome, {fair}");
    }
}

#[test]
fn the_cap_is_configurable_and_unlimited_gives_everything_to_rt() {
    let tasks = [hog(SchedParams::fifo(50)), hog(SchedParams::fair(0))];
    let half = RtBudget {
        period_ns: 100_000_000,
        runtime_ns: 50_000_000,
    };
    let mut capped = Rt::with_config(half, EevdfConfig::default());
    let report = run(&mut capped, &tasks, 5 * SECOND, Timer::OneShot);
    assert!((report.share(0) - 0.5).abs() < 0.005, "{}", report.share(0));
    let mut free = Rt::with_config(RtBudget::UNLIMITED, EevdfConfig::default());
    let report = run(&mut free, &tasks, 5 * SECOND, Timer::OneShot);
    assert_eq!(report.cpu_ns[1], 0);
}

#[test]
fn rt_runs_flat_out_when_nobody_in_the_fair_class_wants_the_cpu() {
    let report = run(
        &mut Rt::new(),
        &[hog(SchedParams::fifo(50))],
        3 * SECOND,
        Timer::OneShot,
    );
    assert_eq!(report.idle_ns, 0);
    assert!((report.share(0) - 1.0).abs() < 1e-9);
}

#[test]
fn a_throttled_fifo_task_resumes_when_the_window_turns() {
    let mut sched = Rt::with_config(
        RtBudget {
            period_ns: 1_000,
            runtime_ns: 600,
        },
        EevdfConfig::default(),
    );
    sched.add(TaskId::new(1), SchedParams::fifo(5), 0).unwrap();
    sched.add(TaskId::new(2), SchedParams::fair(0), 0).unwrap();
    assert_eq!(sched.pick_next(0), Some(TaskId::new(1)));
    assert_eq!(sched.next_deadline_ns(0), Some(600));
    assert!(sched.tick(600));
    assert!(sched.rt_throttled());
    assert_eq!(sched.pick_next(600), Some(TaskId::new(2)));
    assert_eq!(sched.next_deadline_ns(600), Some(1_000));
    assert!(sched.tick(1_000));
    assert!(!sched.rt_throttled());
    assert_eq!(sched.pick_next(1_000), Some(TaskId::new(1)));
}

#[test]
fn the_composite_routes_by_class_and_rejects_duplicates_across_classes() {
    let mut sched = Composite::<2, 2>::new();
    sched.add(TaskId::new(1), SchedParams::fifo(1), 0).unwrap();
    sched.add(TaskId::new(2), SchedParams::fair(0), 0).unwrap();
    assert_eq!(
        sched.add(TaskId::new(1), SchedParams::fair(0), 0),
        Err(crate::SchedError::Duplicate)
    );
    sched.add(TaskId::new(3), SchedParams::fair(0), 0).unwrap();
    assert_eq!(
        sched.add(TaskId::new(4), SchedParams::fair(0), 0),
        Err(crate::SchedError::Full)
    );
    sched.add(TaskId::new(4), SchedParams::fifo(2), 0).unwrap();
    sched.remove(TaskId::new(99));
    assert_eq!(sched.pick_next(0), Some(TaskId::new(4)));
    sched.remove(TaskId::new(4));
    assert_eq!(sched.pick_next(1), Some(TaskId::new(1)));
}
