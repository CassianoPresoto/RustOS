//! Escalonador composto: `Fifo` sempre antes de `Eevdf`, com teto de CPU para o
//! tempo real (`sched_rt_runtime_us` do Linux). Regras e fontes no SDD 14.

use crate::TaskId;
use crate::eevdf::{Eevdf, EevdfConfig};
use crate::fifo::Fifo;
use crate::scheduler::{SchedError, SchedParams, Scheduler};

/// Quanto do tempo o tempo real pode usar por janela, quando a justa tem trabalho.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtBudget {
    pub period_ns: u64,
    pub runtime_ns: u64,
}

impl RtBudget {
    /// Os padrões documentados do Linux: 950 ms por janela de 1 s (95%).
    pub const LINUX: Self = Self {
        period_ns: 1_000_000_000,
        runtime_ns: 950_000_000,
    };
    /// Sem teto: `runtime >= period`, como `sched_rt_runtime_us = -1`.
    pub const UNLIMITED: Self = Self {
        period_ns: 1_000_000_000,
        runtime_ns: 1_000_000_000,
    };

    const fn capped(self) -> bool {
        self.period_ns > 0 && self.runtime_ns < self.period_ns
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Running {
    Idle,
    Fifo,
    Fair,
}

#[derive(Debug)]
pub struct Composite<const RT: usize, const FAIR: usize> {
    fifo: Fifo<RT>,
    fair: Eevdf<FAIR>,
    budget: RtBudget,
    window_start_ns: u64,
    last_ns: u64,
    used_ns: u64,
    throttled: bool,
}

impl<const RT: usize, const FAIR: usize> Composite<RT, FAIR> {
    pub const fn new() -> Self {
        Self::with_config(
            RtBudget::LINUX,
            EevdfConfig {
                base_slice_ns: crate::eevdf::BASE_SLICE_NS,
            },
        )
    }

    pub const fn with_config(budget: RtBudget, fair: EevdfConfig) -> Self {
        Self {
            fifo: Fifo::new(),
            fair: Eevdf::with_config(fair),
            budget,
            window_start_ns: 0,
            last_ns: 0,
            used_ns: 0,
            throttled: false,
        }
    }

    pub const fn fair(&self) -> &Eevdf<FAIR> {
        &self.fair
    }

    pub const fn fifo(&self) -> &Fifo<RT> {
        &self.fifo
    }

    /// O tempo real está bloqueado pelo teto e a justa tem trabalho.
    pub const fn rt_throttled(&self) -> bool {
        self.throttled
    }

    fn running(&self) -> Running {
        if self.fifo.current().is_some() {
            Running::Fifo
        } else if self.fair.current().is_some() {
            Running::Fair
        } else {
            Running::Idle
        }
    }

    /// Cobra o tempo de CPU do tempo real na janela e vira a janela quando ela acaba.
    fn account(&mut self, now_ns: u64) {
        if now_ns <= self.last_ns {
            return;
        }
        let rt_running = self.fifo.current().is_some();
        if self.budget.capped() {
            let window_end = self.window_start_ns.saturating_add(self.budget.period_ns);
            if now_ns >= window_end {
                let windows = (now_ns - self.window_start_ns) / self.budget.period_ns;
                self.window_start_ns += windows * self.budget.period_ns;
                self.used_ns = if rt_running {
                    now_ns - self.last_ns.max(self.window_start_ns)
                } else {
                    0
                };
            } else if rt_running {
                self.used_ns += now_ns - self.last_ns;
            }
            self.throttled = self.used_ns >= self.budget.runtime_ns;
        }
        self.last_ns = now_ns;
    }

    fn rt_blocked(&self) -> bool {
        self.throttled && self.fair.has_runnable()
    }
}

impl<const RT: usize, const FAIR: usize> Default for Composite<RT, FAIR> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const RT: usize, const FAIR: usize> Scheduler for Composite<RT, FAIR> {
    fn add(&mut self, task: TaskId, params: SchedParams, now_ns: u64) -> Result<(), SchedError> {
        if self.fifo.contains(task) || self.fair.contains(task) {
            return Err(SchedError::Duplicate);
        }
        self.account(now_ns);
        match params {
            SchedParams::Fifo { .. } => self.fifo.add(task, params, now_ns),
            SchedParams::Fair { .. } => self.fair.add(task, params, now_ns),
        }
    }

    fn remove(&mut self, task: TaskId) {
        self.fifo.remove(task);
        self.fair.remove(task);
    }

    fn wake(&mut self, task: TaskId, now_ns: u64) {
        self.account(now_ns);
        self.fifo.wake(task, now_ns);
        self.fair.wake(task, now_ns);
    }

    fn sleep(&mut self, task: TaskId, now_ns: u64) {
        self.account(now_ns);
        self.fifo.sleep(task, now_ns);
        self.fair.sleep(task, now_ns);
    }

    fn pick_next(&mut self, now_ns: u64) -> Option<TaskId> {
        self.account(now_ns);
        if self.fifo.has_runnable() && !self.rt_blocked() {
            self.fair.put_prev(now_ns);
            return self.fifo.pick_next(now_ns);
        }
        self.fifo.put_prev();
        self.fair.pick_next(now_ns)
    }

    fn tick(&mut self, now_ns: u64) -> bool {
        self.account(now_ns);
        match self.running() {
            Running::Fifo => {
                self.fifo.tick(now_ns);
            }
            Running::Fair => {
                self.fair.tick(now_ns);
            }
            Running::Idle => {}
        }
        self.needs_resched()
    }

    fn yield_current(&mut self, now_ns: u64) {
        self.account(now_ns);
        match self.running() {
            Running::Fifo => self.fifo.yield_current(now_ns),
            Running::Fair => self.fair.yield_current(now_ns),
            Running::Idle => {}
        }
    }

    fn next_deadline_ns(&self, now_ns: u64) -> Option<u64> {
        let window_end = self.window_start_ns.saturating_add(self.budget.period_ns);
        let mut next: Option<u64> = None;
        let mut sooner = |at: u64| next = Some(next.map_or(at, |n| n.min(at)));
        match self.running() {
            Running::Fifo => {
                if self.budget.capped() && self.fair.has_runnable() {
                    let remaining = self.budget.runtime_ns.saturating_sub(self.used_ns);
                    sooner(self.last_ns.saturating_add(remaining).min(window_end));
                }
            }
            Running::Fair => {
                if let Some(at) = self.fair.next_deadline_ns(now_ns) {
                    sooner(at);
                }
                if self.budget.capped() && self.fifo.has_waiting() && self.throttled {
                    sooner(window_end);
                }
            }
            Running::Idle => {}
        }
        next.map(|at| at.max(now_ns))
    }

    fn needs_resched(&self) -> bool {
        match self.running() {
            Running::Idle => self.fifo.has_runnable() || self.fair.has_runnable(),
            Running::Fifo => self.fifo.needs_resched() || self.rt_blocked(),
            Running::Fair => {
                self.fifo.has_waiting() && !self.rt_blocked() || self.fair.needs_resched()
            }
        }
    }

    fn current(&self) -> Option<TaskId> {
        self.fifo.current().or_else(|| self.fair.current())
    }
}
