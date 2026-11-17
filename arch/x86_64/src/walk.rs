//! Caminhada de 4 níveis na tabela de páginas, sobre uma memória de tabelas abstrata.
//! A memória real (mapa direto) mora em `mmu`; os testes usam uma falsa.

use rustos_hal::{
    FrameSource, LARGE_PAGE_SIZE, MapError, Mapping, PAGE_SIZE, PageFlags, PhysAddr, VirtAddr,
};

use crate::pte::{
    ENTRIES, LARGE_LEVEL, LEVELS, OUTPUT_ADDRESS, canonical, decode_leaf, is_huge, is_present,
    large_entry, level_span, page_entry, restrict, table_entry, table_index,
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
    map_leaf(
        memory,
        root,
        virt,
        frames,
        LEVELS - 1,
        page_entry(phys, flags),
    )
}

/// Folha de 2 MiB no nível 2: `virt` e `phys` alinhados a `LARGE_PAGE_SIZE`.
pub fn map_large_page<M: TableMemory, F: FrameSource>(
    memory: &mut M,
    root: PhysAddr,
    virt: VirtAddr,
    phys: PhysAddr,
    flags: PageFlags,
    frames: &mut F,
) -> Result<(), MapError> {
    if !virt.is_aligned(LARGE_PAGE_SIZE) || !phys.is_aligned(LARGE_PAGE_SIZE) {
        return Err(MapError::Misaligned);
    }
    map_leaf(
        memory,
        root,
        virt,
        frames,
        LARGE_LEVEL,
        large_entry(phys, flags),
    )
}

/// Desce criando tabelas até `leaf_level` e grava `entry` lá.
fn map_leaf<M: TableMemory, F: FrameSource>(
    memory: &mut M,
    root: PhysAddr,
    virt: VirtAddr,
    frames: &mut F,
    leaf_level: usize,
    entry: u64,
) -> Result<(), MapError> {
    let mut table = root;
    for level in 0..leaf_level {
        let index = table_index(virt, level);
        let current = memory.read(table, index);
        table = if !is_present(current) {
            let next = frames.allocate_frame().ok_or(MapError::OutOfFrames)?;
            memory.zero(next);
            memory.write(table, index, table_entry(next));
            next
        } else if level > 0 && is_huge(current) {
            return Err(MapError::HugePageInTheWay(virt));
        } else {
            PhysAddr::new(current & OUTPUT_ADDRESS)
        };
    }
    let index = table_index(virt, leaf_level);
    if is_present(memory.read(table, index)) {
        return Err(MapError::AlreadyMapped(virt));
    }
    memory.write(table, index, entry);
    Ok(())
}

/// Chama `visit` em cada folha, com a permissão efetiva (a de todos os níveis).
pub fn walk<M: TableMemory>(memory: &M, root: PhysAddr, visit: &mut dyn FnMut(Mapping)) {
    walk_table(memory, root, 0, 0, true, false, visit);
}

fn walk_table<M: TableMemory>(
    memory: &M,
    table: PhysAddr,
    level: usize,
    base: u64,
    writable: bool,
    no_execute: bool,
    visit: &mut dyn FnMut(Mapping),
) {
    for index in 0..ENTRIES {
        let entry = memory.read(table, index);
        if !is_present(entry) {
            continue;
        }
        let virt = canonical(base + index as u64 * level_span(level));
        let (writable, no_execute) = restrict(writable, no_execute, entry);
        let leaf = level == LEVELS - 1 || (level > 0 && is_huge(entry));
        if leaf {
            let (phys, mut flags) = decode_leaf(entry, level != LEVELS - 1);
            flags.writable = writable;
            flags.executable = !no_execute;
            visit(Mapping {
                virt: VirtAddr::new(virt),
                phys,
                size: level_span(level),
                flags,
            });
        } else {
            let next = PhysAddr::new(entry & OUTPUT_ADDRESS);
            walk_table(
                memory,
                next,
                level + 1,
                base + index as u64 * level_span(level),
                writable,
                no_execute,
                visit,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::pte::{HUGE, PRESENT};

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

    const LARGE_VIRT: VirtAddr = VirtAddr::new(0xffff_8000_0020_0000);

    #[test]
    fn large_page_stops_one_level_above_the_small_page() {
        let mut tables = FakeTables::default();
        let mut frames = Frames(vec![0x3000, 0x2000]);
        let phys = PhysAddr::new(0x4020_0000);
        map_large_page(
            &mut tables,
            ROOT,
            LARGE_VIRT,
            phys,
            PageFlags::DATA,
            &mut frames,
        )
        .unwrap();
        assert_eq!(frames.0, [] as [u64; 0]);
        let directory = tables.read(PhysAddr::new(0x2000), table_index(LARGE_VIRT, 1));
        assert_eq!(directory, table_entry(PhysAddr::new(0x3000)));
        let leaf = tables.read(PhysAddr::new(0x3000), table_index(LARGE_VIRT, 2));
        assert_eq!(leaf, large_entry(phys, PageFlags::DATA));
    }

    #[test]
    fn large_page_refuses_unaligned_addresses_and_overlaps() {
        let mut tables = FakeTables::default();
        let mut frames = Frames(vec![0x5000, 0x4000, 0x3000, 0x2000]);
        let phys = PhysAddr::new(0x4020_0000);
        let odd = VirtAddr::new(LARGE_VIRT.as_u64() + PAGE_SIZE);
        let data = PageFlags::DATA;
        assert_eq!(
            map_large_page(&mut tables, ROOT, odd, phys, data, &mut frames),
            Err(MapError::Misaligned)
        );
        map_large_page(&mut tables, ROOT, LARGE_VIRT, phys, data, &mut frames).unwrap();
        assert_eq!(
            map_large_page(&mut tables, ROOT, LARGE_VIRT, phys, data, &mut frames),
            Err(MapError::AlreadyMapped(LARGE_VIRT))
        );
        assert_eq!(
            map_page(&mut tables, ROOT, LARGE_VIRT, phys, data, &mut frames),
            Err(MapError::HugePageInTheWay(LARGE_VIRT))
        );
    }

    #[test]
    fn large_page_refuses_a_directory_with_small_pages() {
        let mut tables = FakeTables::default();
        let mut frames = Frames(vec![0x5000, 0x4000, 0x3000, 0x2000]);
        let data = PageFlags::DATA;
        map_page(&mut tables, ROOT, LARGE_VIRT, DEVICE, data, &mut frames).unwrap();
        let phys = PhysAddr::new(0x4020_0000);
        assert_eq!(
            map_large_page(&mut tables, ROOT, LARGE_VIRT, phys, data, &mut frames),
            Err(MapError::AlreadyMapped(LARGE_VIRT))
        );
    }

    fn collect(tables: &FakeTables) -> Vec<Mapping> {
        let mut found = Vec::new();
        walk(tables, ROOT, &mut |mapping| found.push(mapping));
        found
    }

    #[test]
    fn walk_reports_every_leaf_with_its_flags_in_address_order() {
        let mut tables = FakeTables::default();
        let mut frames = Frames((2..10).rev().map(|n| n * 0x1000).collect());
        let code = VirtAddr::new(0xffff_ffff_8000_0000);
        let big = PhysAddr::new(0x4020_0000);
        map_page(
            &mut tables,
            ROOT,
            code,
            PhysAddr::new(0x20_0000),
            PageFlags::CODE,
            &mut frames,
        )
        .unwrap();
        map_large_page(
            &mut tables,
            ROOT,
            LARGE_VIRT,
            big,
            PageFlags::DATA,
            &mut frames,
        )
        .unwrap();
        let found = collect(&tables);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].virt, LARGE_VIRT);
        assert_eq!((found[0].phys, found[0].size), (big, LARGE_PAGE_SIZE));
        assert_eq!(found[0].flags, PageFlags::DATA);
        assert_eq!(found[1].virt, code);
        assert_eq!(
            (found[1].phys.as_u64(), found[1].size),
            (0x20_0000, PAGE_SIZE)
        );
        assert_eq!(found[1].flags, PageFlags::CODE);
    }

    #[test]
    fn walk_applies_the_restrictions_of_the_upper_levels() {
        let mut tables = FakeTables::default();
        let upper = table_entry(PhysAddr::new(0x2000)) & !crate::pte::WRITABLE;
        tables.write(ROOT, 300, upper);
        tables.write(PhysAddr::new(0x2000), 0, table_entry(PhysAddr::new(0x3000)));
        tables.write(PhysAddr::new(0x3000), 0, table_entry(PhysAddr::new(0x4000)));
        tables.write(
            PhysAddr::new(0x4000),
            0,
            page_entry(DEVICE, PageFlags::DATA),
        );
        let found = collect(&tables);
        assert_eq!(found.len(), 1);
        assert!(!found[0].flags.writable);
    }

    #[test]
    fn walk_of_an_empty_root_finds_nothing() {
        assert_eq!(collect(&FakeTables::default()).len(), 0);
    }

    #[test]
    fn rejects_misaligned_addresses() {
        let mut tables = FakeTables::default();
        let odd = VirtAddr::new(VIRT.as_u64() + 1);
        let result = map(&mut tables, odd, &mut Frames(vec![]));
        assert_eq!(result, Err(MapError::Misaligned));
    }
}
