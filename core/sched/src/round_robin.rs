//! Rodízio simples: a referência e o fallback, até a medição pedir outra política.

use crate::scheduler::{SchedError, SchedParams, Scheduler};
use crate::table::IdTable;
use crate::{RunQueue, TaskId};

/// Fatia padrão do rodízio, em ns.
pub const DEFAULT_SLICE_NS: u64 = 4_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Sleeping,
    Ready,
    Running,
}

#[derive(Debug)]
pub struct RoundRobin<const N: usize> {
    table: IdTable<N>,
    states: [State; N],
    ready: RunQueue<N>,
    current: Option<TaskId>,
    slice_ns: u64,
    slice_start_ns: u64,
    yielded: bool,
    expired: bool,
}

impl<const N: usize> RoundRobin<N> {
    pub const fn new() -> Self {
        Self::with_slice(DEFAULT_SLICE_NS)
    }

    pub const fn with_slice(slice_ns: u64) -> Self {
        Self {
            table: IdTable::new(),
            states: [State::Sleeping; N],
            ready: RunQueue::new(),
            current: None,
            slice_ns,
            slice_start_ns: 0,
            yielded: false,
            expired: false,
        }
    }

    fn leave(&mut self, task: TaskId, slot: usize) {
        match self.states[slot] {
            State::Ready => {
                self.ready.remove(task);
            }
            State::Running => self.current = None,
            State::Sleeping => {}
        }
    }
}

impl<const N: usize> Default for RoundRobin<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Scheduler for RoundRobin<N> {
    fn add(&mut self, task: TaskId, _params: SchedParams, _now_ns: u64) -> Result<(), SchedError> {
        let slot = self.table.claim(task)?;
        self.states[slot] = State::Ready;
        let _ = self.ready.push_back(task);
        Ok(())
    }

    fn remove(&mut self, task: TaskId) {
        if let Some(slot) = self.table.find(task) {
            self.leave(task, slot);
            self.table.release(slot);
        }
    }

    fn wake(&mut self, task: TaskId, _now_ns: u64) {
        if let Some(slot) = self.table.find(task)
            && self.states[slot] == State::Sleeping
        {
            self.states[slot] = State::Ready;
            let _ = self.ready.push_back(task);
        }
    }

    fn sleep(&mut self, task: TaskId, _now_ns: u64) {
        if let Some(slot) = self.table.find(task) {
            self.leave(task, slot);
            self.states[slot] = State::Sleeping;
        }
    }

    fn pick_next(&mut self, now_ns: u64) -> Option<TaskId> {
        self.yielded = false;
        self.expired = false;
        if let Some(previous) = self.current.take()
            && let Some(slot) = self.table.find(previous)
        {
            self.states[slot] = State::Ready;
            let _ = self.ready.push_back(previous);
        }
        let next = self.ready.pop_front()?;
        if let Some(slot) = self.table.find(next) {
            self.states[slot] = State::Running;
        }
        self.current = Some(next);
        self.slice_start_ns = now_ns;
        self.current
    }

    fn tick(&mut self, now_ns: u64) -> bool {
        if !self.ready.is_empty() && now_ns.saturating_sub(self.slice_start_ns) >= self.slice_ns {
            self.expired = true;
        }
        self.needs_resched()
    }

    fn yield_current(&mut self, _now_ns: u64) {
        self.yielded = self.current.is_some();
    }

    fn next_deadline_ns(&self, now_ns: u64) -> Option<u64> {
        if self.current.is_some() && !self.ready.is_empty() {
            return Some((self.slice_start_ns + self.slice_ns).max(now_ns));
        }
        None
    }

    fn needs_resched(&self) -> bool {
        self.yielded || self.expired || self.current.is_none() && !self.ready.is_empty()
    }

    fn current(&self) -> Option<TaskId> {
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u32) -> TaskId {
        TaskId::new(raw)
    }

    fn rotate(policy: &mut RoundRobin<4>, now: &mut u64) -> Option<u32> {
        *now += DEFAULT_SLICE_NS;
        policy.pick_next(*now).map(TaskId::as_u32)
    }

    #[test]
    fn rotates_through_admitted_tasks() {
        let mut policy = RoundRobin::<4>::new();
        for raw in 1..=3 {
            policy.add(id(raw), SchedParams::fair(0), 0).unwrap();
        }
        let mut now = 0;
        let order: Vec<_> = (0..5)
            .filter_map(|_| rotate(&mut policy, &mut now))
            .collect();
        assert_eq!(order, [1, 2, 3, 1, 2]);
    }

    #[test]
    fn idle_when_nothing_is_ready() {
        let mut policy = RoundRobin::<4>::new();
        assert_eq!(policy.pick_next(0), None);
        assert_eq!(policy.current(), None);
        assert!(!policy.needs_resched());
        assert_eq!(policy.next_deadline_ns(0), None);
    }

    #[test]
    fn single_task_keeps_running_without_a_timer() {
        let mut policy = RoundRobin::<1>::new();
        policy.add(id(7), SchedParams::fair(0), 0).unwrap();
        assert_eq!(policy.pick_next(0), Some(id(7)));
        assert!(!policy.tick(100 * DEFAULT_SLICE_NS));
        assert_eq!(policy.pick_next(1), Some(id(7)));
        assert_eq!(policy.next_deadline_ns(1), None);
    }

    #[test]
    fn tick_asks_for_preemption_only_after_the_slice_and_with_company() {
        let mut policy = RoundRobin::<4>::with_slice(1_000);
        policy.add(id(1), SchedParams::fair(0), 0).unwrap();
        policy.add(id(2), SchedParams::fair(0), 0).unwrap();
        assert_eq!(policy.pick_next(0), Some(id(1)));
        assert!(!policy.tick(999));
        assert_eq!(policy.next_deadline_ns(500), Some(1_000));
        assert!(policy.tick(1_000));
    }

    #[test]
    fn sleeping_and_waking_move_a_task_out_of_and_into_the_queue() {
        let mut policy = RoundRobin::<4>::new();
        policy.add(id(1), SchedParams::fair(0), 0).unwrap();
        policy.add(id(2), SchedParams::fair(0), 0).unwrap();
        policy.sleep(id(2), 0);
        assert_eq!(policy.pick_next(0), Some(id(1)));
        assert_eq!(policy.pick_next(1), Some(id(1)));
        policy.wake(id(2), 2);
        policy.wake(id(2), 2);
        assert_eq!(policy.pick_next(3), Some(id(2)));
    }

    #[test]
    fn removing_the_current_task_leaves_the_cpu_for_the_next() {
        let mut policy = RoundRobin::<4>::new();
        policy.add(id(1), SchedParams::fair(0), 0).unwrap();
        policy.add(id(2), SchedParams::fair(0), 0).unwrap();
        assert_eq!(policy.pick_next(0), Some(id(1)));
        policy.remove(id(1));
        assert_eq!(policy.current(), None);
        assert!(policy.needs_resched());
        assert_eq!(policy.pick_next(1), Some(id(2)));
        policy.remove(id(2));
        policy.remove(id(2));
        assert_eq!(policy.pick_next(2), None);
    }

    #[test]
    fn rejects_duplicates_and_a_full_table_and_ignores_unknown_ids() {
        let mut policy = RoundRobin::<2>::new();
        policy.add(id(1), SchedParams::fair(0), 0).unwrap();
        assert_eq!(
            policy.add(id(1), SchedParams::fair(0), 0),
            Err(SchedError::Duplicate)
        );
        policy.add(id(2), SchedParams::fair(0), 0).unwrap();
        assert_eq!(
            policy.add(id(3), SchedParams::fair(0), 0),
            Err(SchedError::Full)
        );
        policy.wake(id(99), 0);
        policy.sleep(id(99), 0);
        policy.remove(id(99));
        assert_eq!(policy.pick_next(0), Some(id(1)));
    }

    #[test]
    fn yield_hands_the_cpu_to_the_next_task() {
        let mut policy = RoundRobin::<4>::new();
        policy.add(id(1), SchedParams::fair(0), 0).unwrap();
        policy.add(id(2), SchedParams::fair(0), 0).unwrap();
        assert_eq!(policy.pick_next(0), Some(id(1)));
        policy.yield_current(1);
        assert!(policy.needs_resched());
        assert_eq!(policy.pick_next(1), Some(id(2)));
        assert!(!policy.needs_resched());
    }
}
