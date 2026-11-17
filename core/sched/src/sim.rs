//! Simulação de CPU única para os testes: o laço do kernel (`tick`, `needs_resched`,
//! `pick_next`) contra tarefas que só gastam tempo, sem relógio de verdade.

use crate::TaskId;
use crate::scheduler::{SchedParams, Scheduler};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Kind {
    Hog,
    /// Roda `run_ns`, bloqueia `sleep_ns`, e repete.
    Burst {
        run_ns: u64,
        sleep_ns: u64,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SimTask {
    pub(crate) params: SchedParams,
    pub(crate) kind: Kind,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Timer {
    Periodic(u64),
    OneShot,
}

#[derive(Debug, Default)]
pub(crate) struct Report {
    pub(crate) cpu_ns: Vec<u64>,
    pub(crate) wake_latency_ns: Vec<Vec<u64>>,
    pub(crate) idle_ns: u64,
    pub(crate) switches: u64,
}

#[allow(
    clippy::cast_precision_loss,
    reason = "razão de ns em teste; 52 bits bastam"
)]
pub(crate) fn ratio(numerator: u64, denominator: u64) -> f64 {
    numerator as f64 / denominator.max(1) as f64
}

impl Report {
    pub(crate) fn share(&self, task: usize) -> f64 {
        let total: u64 = self.cpu_ns.iter().sum::<u64>() + self.idle_ns;
        ratio(self.cpu_ns[task], total)
    }

    pub(crate) fn mean_latency_ns(&self, task: usize) -> f64 {
        let samples = &self.wake_latency_ns[task];
        ratio(samples.iter().sum::<u64>(), samples.len() as u64)
    }

    pub(crate) fn max_latency_ns(&self, task: usize) -> u64 {
        self.wake_latency_ns[task]
            .iter()
            .copied()
            .max()
            .unwrap_or(0)
    }
}

struct Runtime {
    id: TaskId,
    kind: Kind,
    awake: bool,
    burst_left: u64,
    wake_at: u64,
    woke_at: Option<u64>,
}

/// Roda `duration_ns` de tempo simulado. A tarefa `i` tem `TaskId::new(i + 1)`.
pub(crate) fn run<S: Scheduler>(
    sched: &mut S,
    tasks: &[SimTask],
    duration_ns: u64,
    timer: Timer,
) -> Report {
    let mut runtimes: Vec<Runtime> = Vec::new();
    for (i, task) in tasks.iter().enumerate() {
        let id = TaskId::new(u32::try_from(i + 1).unwrap());
        sched.add(id, task.params, 0).unwrap();
        let burst_left = match task.kind {
            Kind::Hog => u64::MAX,
            Kind::Burst { run_ns, .. } => run_ns,
        };
        runtimes.push(Runtime {
            id,
            kind: task.kind,
            awake: true,
            burst_left,
            wake_at: 0,
            woke_at: None,
        });
    }
    let mut report = Report {
        cpu_ns: vec![0; tasks.len()],
        wake_latency_ns: vec![Vec::new(); tasks.len()],
        ..Report::default()
    };
    let (mut now, mut current, mut next_tick) = (0_u64, None::<usize>, 0_u64);
    if let Timer::Periodic(period) = timer {
        next_tick = period;
    }
    for _ in 0..200_000_000_u64 {
        if now >= duration_ns {
            return report;
        }
        for rt in runtimes
            .iter_mut()
            .filter(|rt| !rt.awake && rt.wake_at <= now)
        {
            rt.awake = true;
            rt.woke_at = Some(now);
            if let Kind::Burst { run_ns, .. } = rt.kind {
                rt.burst_left = run_ns;
            }
            sched.wake(rt.id, now);
        }
        if current.is_none() || sched.needs_resched() {
            let picked = sched.pick_next(now);
            let next = picked.and_then(|id| runtimes.iter().position(|rt| rt.id == id));
            assert_eq!(sched.current(), picked);
            if next != current {
                report.switches += 1;
            }
            current = next;
            if let Some(i) = current
                && let Some(woke) = runtimes[i].woke_at.take()
            {
                report.wake_latency_ns[i].push(now - woke);
            }
        }
        let timer_at = match timer {
            Timer::Periodic(_) => next_tick,
            Timer::OneShot => sched.next_deadline_ns(now).unwrap_or(u64::MAX),
        };
        let next_wake = runtimes
            .iter()
            .filter(|rt| !rt.awake)
            .map(|rt| rt.wake_at)
            .min()
            .unwrap_or(u64::MAX);
        let mut end = duration_ns.min(next_wake).min(timer_at);
        if let Some(i) = current {
            end = end.min(now.saturating_add(runtimes[i].burst_left));
        }
        let end = end.max(now);
        let dt = end - now;
        match current {
            Some(i) => {
                report.cpu_ns[i] += dt;
                runtimes[i].burst_left = runtimes[i].burst_left.saturating_sub(dt);
            }
            None => report.idle_ns += dt,
        }
        now = end;
        if let Some(i) = current
            && runtimes[i].burst_left == 0
            && let Kind::Burst { sleep_ns, .. } = runtimes[i].kind
        {
            sched.sleep(runtimes[i].id, now);
            runtimes[i].awake = false;
            runtimes[i].wake_at = now + sleep_ns;
            current = None;
        }
        if now >= timer_at {
            sched.tick(now);
            if let Timer::Periodic(period) = timer {
                next_tick += period;
            }
        }
    }
    panic!("simulação sem progresso");
}
