//! Tabela `TaskId` para slot, de capacidade fixa. O slot `id % N` é tentado
//! primeiro: com ids densos a busca é O(1).

use crate::TaskId;
use crate::scheduler::SchedError;

#[derive(Debug)]
pub(crate) struct IdTable<const N: usize> {
    ids: [Option<TaskId>; N],
}

impl<const N: usize> IdTable<N> {
    pub(crate) const fn new() -> Self {
        const { assert!(N > 0 && N < u16::MAX as usize) };
        Self { ids: [None; N] }
    }

    pub(crate) fn find(&self, task: TaskId) -> Option<usize> {
        let hint = task.as_u32() as usize % N;
        if self.ids[hint] == Some(task) {
            return Some(hint);
        }
        self.ids.iter().position(|slot| *slot == Some(task))
    }

    pub(crate) fn claim(&mut self, task: TaskId) -> Result<usize, SchedError> {
        if self.find(task).is_some() {
            return Err(SchedError::Duplicate);
        }
        let hint = task.as_u32() as usize % N;
        let slot = if self.ids[hint].is_none() {
            hint
        } else {
            self.ids
                .iter()
                .position(Option::is_none)
                .ok_or(SchedError::Full)?
        };
        self.ids[slot] = Some(task);
        Ok(slot)
    }

    pub(crate) fn release(&mut self, slot: usize) {
        self.ids[slot] = None;
    }

    pub(crate) fn get(&self, slot: usize) -> Option<TaskId> {
        self.ids.get(slot).copied().flatten()
    }
}

/// Índice de slot em `u16`; `NIL` marca ausência nas listas e na árvore.
pub(crate) const NIL: u16 = u16::MAX;

pub(crate) fn slot_u16(slot: usize) -> u16 {
    u16::try_from(slot).unwrap_or(NIL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_ids_land_on_their_hint_slot() {
        let mut table = IdTable::<4>::new();
        assert_eq!(table.claim(TaskId::new(2)), Ok(2));
        assert_eq!(table.find(TaskId::new(2)), Some(2));
        assert_eq!(table.find(TaskId::new(6)), None);
    }

    #[test]
    fn colliding_ids_take_the_first_free_slot_and_are_found() {
        let mut table = IdTable::<4>::new();
        assert_eq!(table.claim(TaskId::new(1)), Ok(1));
        assert_eq!(table.claim(TaskId::new(5)), Ok(0));
        assert_eq!(table.find(TaskId::new(5)), Some(0));
    }

    #[test]
    fn rejects_duplicate_and_full() {
        let mut table = IdTable::<2>::new();
        table.claim(TaskId::new(0)).unwrap();
        table.claim(TaskId::new(1)).unwrap();
        assert_eq!(table.claim(TaskId::new(1)), Err(SchedError::Duplicate));
        assert_eq!(table.claim(TaskId::new(9)), Err(SchedError::Full));
        table.release(0);
        assert_eq!(table.claim(TaskId::new(9)), Ok(0));
    }
}
