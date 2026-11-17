//! Fila circular de capacidade fixa: sem alocação no caminho do escalonador.

use crate::TaskId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueFull(pub TaskId);

#[derive(Debug)]
pub struct RunQueue<const N: usize> {
    slots: [Option<TaskId>; N],
    head: usize,
    len: usize,
}

impl<const N: usize> RunQueue<N> {
    pub const fn new() -> Self {
        Self {
            slots: [None; N],
            head: 0,
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn push_back(&mut self, task: TaskId) -> Result<(), QueueFull> {
        if self.len == N {
            return Err(QueueFull(task));
        }
        self.slots[(self.head + self.len) % N] = Some(task);
        self.len += 1;
        Ok(())
    }

    /// Tira `task` de onde estiver, preservando a ordem das demais. O(n).
    pub fn remove(&mut self, task: TaskId) -> bool {
        let Some(at) = (0..self.len).find(|i| self.slots[(self.head + i) % N] == Some(task)) else {
            return false;
        };
        for i in at..self.len - 1 {
            self.slots[(self.head + i) % N] = self.slots[(self.head + i + 1) % N];
        }
        self.slots[(self.head + self.len - 1) % N] = None;
        self.len -= 1;
        true
    }

    pub fn pop_front(&mut self) -> Option<TaskId> {
        if self.len == 0 {
            return None;
        }
        let task = self.slots[self.head].take();
        self.head = (self.head + 1) % N;
        self.len -= 1;
        task
    }
}

impl<const N: usize> Default for RunQueue<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_fifo_order_across_wraparound() {
        let mut queue = RunQueue::<2>::new();
        queue.push_back(TaskId::new(1)).unwrap();
        queue.push_back(TaskId::new(2)).unwrap();
        assert_eq!(queue.pop_front(), Some(TaskId::new(1)));
        queue.push_back(TaskId::new(3)).unwrap();
        assert_eq!(queue.pop_front(), Some(TaskId::new(2)));
        assert_eq!(queue.pop_front(), Some(TaskId::new(3)));
        assert!(queue.is_empty());
    }

    #[test]
    fn rejects_push_when_full() {
        let mut queue = RunQueue::<1>::new();
        queue.push_back(TaskId::new(1)).unwrap();
        assert_eq!(
            queue.push_back(TaskId::new(2)),
            Err(QueueFull(TaskId::new(2)))
        );
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn remove_keeps_the_order_of_the_rest() {
        let mut queue = RunQueue::<4>::new();
        for raw in 1..=4 {
            queue.push_back(TaskId::new(raw)).unwrap();
        }
        assert_eq!(queue.pop_front(), Some(TaskId::new(1)));
        queue.push_back(TaskId::new(5)).unwrap();
        assert!(queue.remove(TaskId::new(3)));
        assert!(!queue.remove(TaskId::new(3)));
        let rest: Vec<_> = core::iter::from_fn(|| queue.pop_front()).collect();
        assert_eq!(rest, [TaskId::new(2), TaskId::new(4), TaskId::new(5)]);
    }

    #[test]
    fn pop_on_empty_is_none() {
        assert_eq!(RunQueue::<4>::new().pop_front(), None);
    }
}
