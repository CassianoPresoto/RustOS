//! Tempo real: SCHED_FIFO do Linux, prioridades 1 a 99 (`sched(7)`). Sem fatia: a
//! corrente roda até bloquear, ceder ou ser preemptada por prioridade maior.

use crate::TaskId;
use crate::scheduler::{MAX_FIFO_PRIORITY, MIN_FIFO_PRIORITY, SchedError, SchedParams, Scheduler};
use crate::table::{IdTable, NIL, slot_u16};

const LEVELS: usize = MAX_FIFO_PRIORITY as usize + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Sleeping,
    Ready,
    Running,
}

#[derive(Clone, Copy, Debug)]
struct Task {
    priority: u8,
    state: State,
    prev: u16,
    next: u16,
}

const FRESH: Task = Task {
    priority: MIN_FIFO_PRIORITY,
    state: State::Sleeping,
    prev: NIL,
    next: NIL,
};

#[derive(Debug)]
pub struct Fifo<const N: usize> {
    table: IdTable<N>,
    tasks: [Task; N],
    heads: [u16; LEVELS],
    tails: [u16; LEVELS],
    ready_mask: u128,
    curr: u16,
    yielded: bool,
}

impl<const N: usize> Fifo<N> {
    pub const fn new() -> Self {
        Self {
            table: IdTable::new(),
            tasks: [FRESH; N],
            heads: [NIL; LEVELS],
            tails: [NIL; LEVELS],
            ready_mask: 0,
            curr: NIL,
            yielded: false,
        }
    }

    pub fn contains(&self, task: TaskId) -> bool {
        self.table.find(task).is_some()
    }

    /// Há alguém pronto ou correndo.
    pub const fn has_runnable(&self) -> bool {
        self.curr != NIL || self.ready_mask != 0
    }

    /// Há alguém pronto esperando (a corrente não conta).
    pub const fn has_waiting(&self) -> bool {
        self.ready_mask != 0
    }

    fn task(&self, slot: u16) -> &Task {
        &self.tasks[usize::from(slot)]
    }

    fn task_mut(&mut self, slot: u16) -> &mut Task {
        &mut self.tasks[usize::from(slot)]
    }

    /// Maior prioridade com alguém esperando.
    fn top_waiting(&self) -> Option<u8> {
        if self.ready_mask == 0 {
            return None;
        }
        u8::try_from(self.ready_mask.ilog2()).ok()
    }

    fn push(&mut self, slot: u16, front: bool) {
        let level = usize::from(self.task(slot).priority);
        let (head, tail) = (self.heads[level], self.tails[level]);
        let task = self.task_mut(slot);
        task.state = State::Ready;
        if head == NIL {
            task.prev = NIL;
            task.next = NIL;
            self.heads[level] = slot;
            self.tails[level] = slot;
        } else if front {
            task.prev = NIL;
            task.next = head;
            self.task_mut(head).prev = slot;
            self.heads[level] = slot;
        } else {
            task.prev = tail;
            task.next = NIL;
            self.task_mut(tail).next = slot;
            self.tails[level] = slot;
        }
        self.ready_mask |= 1 << level;
    }

    fn unlink(&mut self, slot: u16) {
        let Task {
            priority,
            prev,
            next,
            ..
        } = *self.task(slot);
        let level = usize::from(priority);
        if prev == NIL {
            self.heads[level] = next;
        } else {
            self.task_mut(prev).next = next;
        }
        if next == NIL {
            self.tails[level] = prev;
        } else {
            self.task_mut(next).prev = prev;
        }
        if self.heads[level] == NIL {
            self.ready_mask &= !(1 << level);
        }
    }

    /// A corrente volta à cabeça da sua prioridade, como em `sched(7)`.
    pub fn put_prev(&mut self) {
        if self.curr != NIL {
            let slot = self.curr;
            self.curr = NIL;
            self.push(slot, true);
        }
    }

    fn leave(&mut self, slot: u16) {
        match self.task(slot).state {
            State::Running => self.curr = NIL,
            State::Ready => self.unlink(slot),
            State::Sleeping => {}
        }
    }
}

impl<const N: usize> Default for Fifo<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Scheduler for Fifo<N> {
    fn add(&mut self, task: TaskId, params: SchedParams, _now_ns: u64) -> Result<(), SchedError> {
        let SchedParams::Fifo { priority } = params else {
            return Err(SchedError::WrongClass);
        };
        if !(MIN_FIFO_PRIORITY..=MAX_FIFO_PRIORITY).contains(&priority) {
            return Err(SchedError::InvalidPriority);
        }
        let slot = slot_u16(self.table.claim(task)?);
        *self.task_mut(slot) = Task { priority, ..FRESH };
        self.push(slot, false);
        Ok(())
    }

    fn remove(&mut self, task: TaskId) {
        if let Some(slot) = self.table.find(task) {
            self.leave(slot_u16(slot));
            self.table.release(slot);
        }
    }

    fn wake(&mut self, task: TaskId, _now_ns: u64) {
        if let Some(slot) = self.table.find(task).map(slot_u16)
            && self.task(slot).state == State::Sleeping
        {
            self.push(slot, false);
        }
    }

    fn sleep(&mut self, task: TaskId, _now_ns: u64) {
        if let Some(slot) = self.table.find(task).map(slot_u16) {
            self.leave(slot);
            self.task_mut(slot).state = State::Sleeping;
        }
    }

    fn pick_next(&mut self, _now_ns: u64) -> Option<TaskId> {
        if self.yielded {
            self.yielded = false;
            if self.curr != NIL {
                let slot = self.curr;
                self.curr = NIL;
                self.push(slot, false);
            }
        }
        if let Some(top) = self.top_waiting()
            && (self.curr == NIL || top > self.task(self.curr).priority)
        {
            self.put_prev();
            let slot = self.heads[usize::from(top)];
            self.unlink(slot);
            self.task_mut(slot).state = State::Running;
            self.curr = slot;
        }
        self.table.get(usize::from(self.curr))
    }

    fn tick(&mut self, _now_ns: u64) -> bool {
        self.needs_resched()
    }

    /// Cede só a quem tem a mesma prioridade (POSIX): nunca desce para menor.
    fn yield_current(&mut self, _now_ns: u64) {
        if self.curr != NIL {
            let level = usize::from(self.task(self.curr).priority);
            self.yielded = self.heads[level] != NIL;
        }
    }

    fn next_deadline_ns(&self, _now_ns: u64) -> Option<u64> {
        None
    }

    fn needs_resched(&self) -> bool {
        match self.top_waiting() {
            None => false,
            Some(top) => {
                self.curr == NIL
                    || top > self.task(self.curr).priority
                    || self.yielded && top == self.task(self.curr).priority
            }
        }
    }

    fn current(&self) -> Option<TaskId> {
        if self.curr == NIL {
            return None;
        }
        self.table.get(usize::from(self.curr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u32) -> TaskId {
        TaskId::new(raw)
    }

    fn fifo(sched: &mut Fifo<8>, raw: u32, priority: u8) {
        sched.add(id(raw), SchedParams::fifo(priority), 0).unwrap();
    }

    #[test]
    fn highest_priority_runs_first_and_equals_run_in_arrival_order() {
        let mut sched = Fifo::<8>::new();
        fifo(&mut sched, 1, 10);
        fifo(&mut sched, 2, 50);
        fifo(&mut sched, 3, 50);
        fifo(&mut sched, 4, 99);
        assert_eq!(sched.pick_next(0), Some(id(4)));
        sched.sleep(id(4), 0);
        assert_eq!(sched.pick_next(0), Some(id(2)));
        assert!(
            !sched.tick(1_000_000_000),
            "sem fatia: a corrente não roda por tempo"
        );
        sched.sleep(id(2), 0);
        assert_eq!(sched.pick_next(0), Some(id(3)));
        sched.sleep(id(3), 0);
        assert_eq!(sched.pick_next(0), Some(id(1)));
    }

    #[test]
    fn a_higher_priority_wakeup_preempts_and_the_victim_resumes_at_the_head() {
        let mut sched = Fifo::<8>::new();
        fifo(&mut sched, 1, 10);
        fifo(&mut sched, 2, 10);
        assert_eq!(sched.pick_next(0), Some(id(1)));
        fifo(&mut sched, 3, 20);
        assert!(sched.needs_resched());
        assert_eq!(sched.pick_next(1), Some(id(3)));
        sched.sleep(id(3), 2);
        assert_eq!(sched.pick_next(2), Some(id(1)), "volta à frente do 2");
    }

    #[test]
    fn an_equal_priority_wakeup_does_not_preempt_and_waits_at_the_tail() {
        let mut sched = Fifo::<8>::new();
        fifo(&mut sched, 1, 10);
        fifo(&mut sched, 2, 10);
        assert_eq!(sched.pick_next(0), Some(id(1)));
        sched.sleep(id(2), 1);
        sched.wake(id(2), 2);
        assert!(!sched.needs_resched());
        assert_eq!(sched.pick_next(3), Some(id(1)));
    }

    #[test]
    fn yield_rotates_within_the_priority_but_never_gives_way_to_a_lower_one() {
        let mut sched = Fifo::<8>::new();
        fifo(&mut sched, 1, 10);
        fifo(&mut sched, 2, 10);
        fifo(&mut sched, 3, 5);
        assert_eq!(sched.pick_next(0), Some(id(1)));
        sched.yield_current(1);
        assert!(sched.needs_resched());
        assert_eq!(sched.pick_next(1), Some(id(2)));
        sched.sleep(id(1), 2);
        sched.yield_current(2);
        assert!(!sched.needs_resched());
        assert_eq!(sched.pick_next(2), Some(id(2)));
    }

    #[test]
    fn rejects_bad_priority_wrong_class_duplicates_and_a_full_table() {
        let mut sched = Fifo::<2>::new();
        assert_eq!(
            sched.add(id(1), SchedParams::fifo(0), 0),
            Err(SchedError::InvalidPriority)
        );
        assert_eq!(
            sched.add(id(1), SchedParams::fifo(100), 0),
            Err(SchedError::InvalidPriority)
        );
        assert_eq!(
            sched.add(id(1), SchedParams::fair(0), 0),
            Err(SchedError::WrongClass)
        );
        sched.add(id(1), SchedParams::fifo(1), 0).unwrap();
        sched.add(id(2), SchedParams::fifo(99), 0).unwrap();
        assert_eq!(
            sched.add(id(2), SchedParams::fifo(5), 0),
            Err(SchedError::Duplicate)
        );
        assert_eq!(
            sched.add(id(3), SchedParams::fifo(5), 0),
            Err(SchedError::Full)
        );
    }

    #[test]
    fn idle_unknown_ids_and_removing_the_current() {
        let mut sched = Fifo::<4>::new();
        assert_eq!(sched.pick_next(0), None);
        assert!(!sched.needs_resched());
        sched.wake(id(9), 0);
        sched.sleep(id(9), 0);
        sched.remove(id(9));
        sched.add(id(1), SchedParams::fifo(7), 0).unwrap();
        sched.add(id(2), SchedParams::fifo(3), 0).unwrap();
        assert_eq!(sched.pick_next(0), Some(id(1)));
        sched.remove(id(1));
        assert_eq!(sched.current(), None);
        assert!(sched.needs_resched());
        assert_eq!(sched.pick_next(1), Some(id(2)));
        sched.sleep(id(2), 2);
        assert_eq!(sched.pick_next(2), None);
        assert_eq!(sched.next_deadline_ns(2), None);
    }

    #[test]
    fn removing_a_ready_task_in_the_middle_of_a_list_keeps_the_links() {
        let mut sched = Fifo::<8>::new();
        for raw in 1..=4 {
            fifo(&mut sched, raw, 10);
        }
        sched.remove(id(2));
        sched.remove(id(4));
        let order: Vec<_> = core::iter::from_fn(|| {
            let next = sched.pick_next(0)?;
            sched.sleep(next, 0);
            Some(next.as_u32())
        })
        .collect();
        assert_eq!(order, [1, 3]);
    }
}
