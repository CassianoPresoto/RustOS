//! Rodízio simples: a política de partida, até existir medição que peça outra.

use crate::{QueueFull, RunQueue, TaskId};

#[derive(Debug, Default)]
pub struct RoundRobin<const N: usize> {
    ready: RunQueue<N>,
    current: Option<TaskId>,
}

impl<const N: usize> RoundRobin<N> {
    pub const fn new() -> Self {
        Self {
            ready: RunQueue::new(),
            current: None,
        }
    }

    pub const fn current(&self) -> Option<TaskId> {
        self.current
    }

    pub fn admit(&mut self, task: TaskId) -> Result<(), QueueFull> {
        self.ready.push_back(task)
    }

    /// Fim da fatia: a tarefa atual volta ao fim da fila e a próxima assume.
    pub fn tick(&mut self) -> Option<TaskId> {
        if let Some(previous) = self.current.take()
            && let Err(QueueFull(task)) = self.ready.push_back(previous)
        {
            self.current = Some(task);
            return self.current;
        }
        self.current = self.ready.pop_front();
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_through_admitted_tasks() {
        let mut policy = RoundRobin::<4>::new();
        for raw in 1..=3 {
            policy.admit(TaskId::new(raw)).unwrap();
        }
        let order: Vec<u32> = (0..5)
            .filter_map(|_| policy.tick())
            .map(TaskId::as_u32)
            .collect();
        assert_eq!(order, [1, 2, 3, 1, 2]);
    }

    #[test]
    fn idle_when_nothing_admitted() {
        let mut policy = RoundRobin::<4>::new();
        assert_eq!(policy.tick(), None);
        assert_eq!(policy.current(), None);
    }

    #[test]
    fn single_task_keeps_running() {
        let mut policy = RoundRobin::<1>::new();
        policy.admit(TaskId::new(7)).unwrap();
        assert_eq!(policy.tick(), Some(TaskId::new(7)));
        assert_eq!(policy.tick(), Some(TaskId::new(7)));
    }
}
