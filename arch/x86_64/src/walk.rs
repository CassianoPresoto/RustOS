//! Caminhada de 4 níveis na tabela de páginas, sobre uma memória de tabelas abstrata.
//! A memória real (mapa direto) mora em `mmu`; os testes usam uma falsa.

use rustos_hal::{FrameSource, MapError, PAGE_SIZE, PageFlags, PhysAddr, VirtAddr};

use crate::pte::{
    LEVELS, OUTPUT_ADDRESS, is_huge, is_present, page_entry, table_entry, table_index,
};

pub trait TableMemory {
    fn read(&self, table: PhysAddr, index: usize) -> u64;
    fn write(&mut self, table: PhysAddr, index: usize, entry: u64);
    fn zero(&mut self, table: PhysAddr);
}

pub fn map_page<M: TableMemory, F: FrameSource>(
    memory: &mut M,
    root: PhysAddr,
    virt: VirtAddr,
    phys: PhysAddr,
    flags: PageFlags,
    frames: &mut F,
) -> Result<(), MapError> {
    if !virt.is_aligned(PAGE_SIZE) || !phys.is_aligned(PAGE_SIZE) {
        return Err(MapError::Misaligned);
    }
    let mut table = root;
    for level in 0..LEVELS - 1 {
        let index = table_index(virt, level);
        let entry = memory.read(table, index);
        table = if !is_present(entry) {
            let next = frames.allocate_frame().ok_or(MapError::OutOfFrames)?;
            memory.zero(next);
            memory.write(table, index, table_entry(next));
            next
        } else if level > 0 && is_huge(entry) {
            return Err(MapError::HugePageInTheWay(virt));
        } else {
            PhysAddr::new(entry & OUTPUT_ADDRESS)
        };
    }
    let index = table_index(virt, LEVELS - 1);
    if is_present(memory.read(table, index)) {
        return Err(MapError::AlreadyMapped(virt));
    }
    memory.write(table, index, page_entry(phys, flags));
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::pte::{ENTRIES, HUGE, PRESENT};

    #[derive(Default)]
    struct FakeTables(HashMap<u64, [u64; ENTRIES]>);

    impl TableMemory for FakeTables {
        fn read(&self, table: PhysAddr, index: usize) -> u64 {
            self.0
                .get(&table.as_u64())
                .map_or(0, |entries| entries[index])
        }

        fn write(&mut self, table: PhysAddr, index: usize, entry: u64) {
            self.0.entry(table.as_u64()).or_insert([0; ENTRIES])[index] = entry;
        }

        fn zero(&mut self, table: PhysAddr) {
            self.0.insert(table.as_u64(), [0; ENTRIES]);
        }
    }

    struct Frames(Vec<u64>);

    impl FrameSource for Frames {
        fn allocate_frame(&mut self) -> Option<PhysAddr> {
            self.0.pop().map(PhysAddr::new)
        }
    }

    const ROOT: PhysAddr = PhysAddr::new(0x1000);
    const VIRT: VirtAddr = VirtAddr::new(0xffff_ff00_0000_0000);
    const DEVICE: PhysAddr = PhysAddr::new(0xFEE0_0000);

    fn map(tables: &mut FakeTables, virt: VirtAddr, frames: &mut Frames) -> Result<(), MapError> {
        map_page(tables, ROOT, virt, DEVICE, PageFlags::DEVICE, frames)
    }

    #[test]
    fn creates_missing_levels_and_writes_leaf() {
        let mut tables = FakeTables::default();
        let mut frames = Frames(vec![0x4000, 0x3000, 0x2000]);
        map(&mut tables, VIRT, &mut frames).unwrap();
        assert_eq!(frames.0, [] as [u64; 0]);
        assert_eq!(
            tables.read(ROOT, table_index(VIRT, 0)),
            table_entry(PhysAddr::new(0x2000))
        );
        let leaf = tables.read(PhysAddr::new(0x4000), table_index(VIRT, 3));
        assert_eq!(leaf, page_entry(DEVICE, PageFlags::DEVICE));
    }

    #[test]
    fn new_intermediate_tables_start_zeroed() {
        let mut tables = FakeTables::default();
        tables.0.insert(0x2000, [u64::MAX; ENTRIES]);
        let mut frames = Frames(vec![0x4000, 0x3000, 0x2000]);
        map(&mut tables, VIRT, &mut frames).unwrap();
        let used = tables.0[&0x2000]
            .iter()
            .filter(|&&entry| entry != 0)
            .count();
        assert_eq!(used, 1);
    }

    #[test]
    fn reuses_existing_tables() {
        let mut tables = FakeTables::default();
        let mut frames = Frames(vec![0x4000, 0x3000, 0x2000]);
        map(&mut tables, VIRT, &mut frames).unwrap();
        let next = VirtAddr::new(VIRT.as_u64() + PAGE_SIZE);
        map(&mut tables, next, &mut frames).unwrap();
        assert_eq!(tables.0.len(), 4);
    }

    #[test]
    fn refuses_to_overwrite_a_mapping() {
        let mut tables = FakeTables::default();
        let mut frames = Frames(vec![0x4000, 0x3000, 0x2000]);
        map(&mut tables, VIRT, &mut frames).unwrap();
        assert_eq!(
            map(&mut tables, VIRT, &mut frames),
            Err(MapError::AlreadyMapped(VIRT))
        );
    }

    #[test]
    fn stops_at_huge_pages() {
        let mut tables = FakeTables::default();
        tables.write(
            ROOT,
            table_index(VIRT, 0),
            table_entry(PhysAddr::new(0x2000)),
        );
        tables.write(PhysAddr::new(0x2000), table_index(VIRT, 1), HUGE | PRESENT);
        let result = map(&mut tables, VIRT, &mut Frames(vec![]));
        assert_eq!(result, Err(MapError::HugePageInTheWay(VIRT)));
    }

    #[test]
    fn reports_exhausted_frames() {
        let mut tables = FakeTables::default();
        let result = map(&mut tables, VIRT, &mut Frames(vec![]));
        assert_eq!(result, Err(MapError::OutOfFrames));
    }

    #[test]
    fn rejects_misaligned_addresses() {
        let mut tables = FakeTables::default();
        let odd = VirtAddr::new(VIRT.as_u64() + 1);
        let result = map(&mut tables, odd, &mut Frames(vec![]));
        assert_eq!(result, Err(MapError::Misaligned));
    }
}
