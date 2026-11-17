//! Caminhada de 4 níveis na tabela de páginas, a mesma nas duas ISAs. A codificação
//! vem de `PageFormat`, a memória das tabelas de `TableMemory` e a CPU de `MmuControl`.

use core::marker::PhantomData;

use rustos_hal::{
    AddressSpace, FrameSource, LARGE_PAGE_SIZE, MapError, Mapping, MmuControl, PAGE_SIZE,
    PageFlags, PageFormat, PageMapper, PhysAddr, TableMemory, VirtAddr,
};

/// Níveis e entradas por tabela: grânulo de 4 KiB, 48 bits, nas duas ISAs.
pub const LEVELS: usize = 4;
pub const ENTRIES: usize = 512;
/// A folha de 2 MiB mora no nível 2.
pub const LARGE_LEVEL: usize = 2;
const _: () = assert!(level_span(LARGE_LEVEL) == LARGE_PAGE_SIZE);
const _: () = assert!(level_span(LEVELS - 1) == PAGE_SIZE);

/// Tamanho do que uma entrada do nível `level` cobre (0 = 512 GiB, 3 = 4 KiB).
pub const fn level_span(level: usize) -> u64 {
    1 << (39 - 9 * level)
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "o índice é mascarado para < 512"
)]
pub const fn table_index(virt: VirtAddr, level: usize) -> usize {
    ((virt.as_u64() >> (39 - 9 * level)) & (ENTRIES as u64 - 1)) as usize
}

/// Um espaço de endereçamento: a raiz, a memória das tabelas e o controle da CPU.
#[derive(Debug)]
pub struct PageTables<Fmt, Mem, Ctl> {
    root: PhysAddr,
    memory: Mem,
    control: Ctl,
    format: PhantomData<fn() -> Fmt>,
}

impl<Fmt: PageFormat, Mem: TableMemory, Ctl: MmuControl> PageTables<Fmt, Mem, Ctl> {
    /// Sobre uma raiz que já existe (as tabelas ativas, por exemplo).
    pub const fn new(root: PhysAddr, memory: Mem, control: Ctl) -> Self {
        Self {
            root,
            memory,
            control,
            format: PhantomData,
        }
    }

    /// Raiz nova e vazia, para montar antes de `activate`.
    pub fn new_empty<F: FrameSource>(
        frames: &mut F,
        mut memory: Mem,
        control: Ctl,
    ) -> Result<Self, MapError> {
        let root = frames.allocate_frame().ok_or(MapError::OutOfFrames)?;
        memory.zero(root);
        Ok(Self::new(root, memory, control))
    }

    /// Desce criando tabelas até `leaf_level` e grava `entry` lá.
    fn map_leaf<F: FrameSource>(
        &mut self,
        virt: VirtAddr,
        frames: &mut F,
        leaf_level: usize,
        entry: u64,
    ) -> Result<(), MapError> {
        let mut table = self.root;
        for level in 0..leaf_level {
            let index = table_index(virt, level);
            let current = self.memory.read(table, index);
            table = if !Fmt::is_valid(current) {
                let next = frames.allocate_frame().ok_or(MapError::OutOfFrames)?;
                self.memory.zero(next);
                self.memory.write(table, index, Fmt::table_entry(next));
                next
            } else if Fmt::is_table(current, level) {
                Fmt::table_address(current)
            } else {
                return Err(MapError::HugePageInTheWay(virt));
            };
        }
        let index = table_index(virt, leaf_level);
        if Fmt::is_valid(self.memory.read(table, index)) {
            return Err(MapError::AlreadyMapped(virt));
        }
        self.memory.write(table, index, entry);
        self.control.entry_changed(virt);
        Ok(())
    }

    fn walk_table(
        &self,
        table: PhysAddr,
        level: usize,
        base: u64,
        permission: (bool, bool),
        visit: &mut dyn FnMut(Mapping),
    ) {
        for index in 0..ENTRIES {
            let entry = self.memory.read(table, index);
            if !Fmt::is_valid(entry) {
                continue;
            }
            let raw = base + index as u64 * level_span(level);
            if level == LEVELS - 1 || !Fmt::is_table(entry, level) {
                let (phys, mut flags) = Fmt::decode_leaf(entry, level != LEVELS - 1);
                flags.writable &= permission.0;
                flags.executable &= !permission.1;
                visit(Mapping {
                    virt: VirtAddr::new(Fmt::canonical(raw)),
                    phys,
                    size: level_span(level),
                    flags,
                });
            } else {
                let below = Fmt::restrict(permission.0, permission.1, entry);
                self.walk_table(Fmt::table_address(entry), level + 1, raw, below, visit);
            }
        }
    }
}

impl<Fmt: PageFormat, Mem: TableMemory, Ctl: MmuControl> PageMapper for PageTables<Fmt, Mem, Ctl> {
    fn map_page<F: FrameSource>(
        &mut self,
        virt: VirtAddr,
        phys: PhysAddr,
        flags: PageFlags,
        frames: &mut F,
    ) -> Result<(), MapError> {
        if flags.violates_wx() {
            return Err(MapError::WriteExecute(virt));
        }
        if !virt.is_aligned(PAGE_SIZE) || !phys.is_aligned(PAGE_SIZE) {
            return Err(MapError::Misaligned);
        }
        self.map_leaf(virt, frames, LEVELS - 1, Fmt::page_entry(phys, flags))
    }

    fn map_large_page<F: FrameSource>(
        &mut self,
        virt: VirtAddr,
        phys: PhysAddr,
        flags: PageFlags,
        frames: &mut F,
    ) -> Result<(), MapError> {
        if flags.violates_wx() {
            return Err(MapError::WriteExecute(virt));
        }
        if !virt.is_aligned(LARGE_PAGE_SIZE) || !phys.is_aligned(LARGE_PAGE_SIZE) {
            return Err(MapError::Misaligned);
        }
        self.map_leaf(virt, frames, LARGE_LEVEL, Fmt::large_entry(phys, flags))
    }
}

impl<Fmt: PageFormat, Mem: TableMemory, Ctl: MmuControl> AddressSpace
    for PageTables<Fmt, Mem, Ctl>
{
    fn activate(&self) {
        self.control.activate(self.root);
    }

    /// A permissão de cada folha é a efetiva: a de todos os níveis acima.
    fn walk(&self, visit: &mut dyn FnMut(Mapping)) {
        self.walk_table(self.root, 0, 0, (true, false), visit);
    }
}

#[cfg(test)]
mod tests {
    use core::cell::RefCell;
    use std::collections::HashMap;

    use super::*;
    use rustos_hal::CacheMode;

    const VALID: u64 = 1;
    const TABLE: u64 = 1 << 1;
    const WRITE: u64 = 1 << 2;
    const NO_EXEC: u64 = 1 << 3;
    const LARGE: u64 = 1 << 7;
    const ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;
    const LARGE_ADDRESS: u64 = 0x000F_FFFF_FFE0_0000;

    /// Formato de brinquedo, com a mesma forma dos reais: bit de tabela, de escrita e
    /// de não-execução.
    struct Toy;

    fn leaf_bits(flags: PageFlags) -> u64 {
        VALID | if flags.writable { WRITE } else { 0 } | if flags.executable { 0 } else { NO_EXEC }
    }

    impl PageFormat for Toy {
        fn table_entry(next: PhysAddr) -> u64 {
            next.as_u64() | VALID | TABLE | WRITE
        }

        fn page_entry(phys: PhysAddr, flags: PageFlags) -> u64 {
            phys.as_u64() | leaf_bits(flags)
        }

        fn large_entry(phys: PhysAddr, flags: PageFlags) -> u64 {
            phys.as_u64() | LARGE | leaf_bits(flags)
        }

        fn is_valid(entry: u64) -> bool {
            entry & VALID != 0
        }

        fn is_table(entry: u64, _level: usize) -> bool {
            entry & TABLE != 0
        }

        fn table_address(entry: u64) -> PhysAddr {
            PhysAddr::new(entry & ADDRESS)
        }

        fn decode_leaf(entry: u64, large: bool) -> (PhysAddr, PageFlags) {
            let mask = if large { LARGE_ADDRESS } else { ADDRESS };
            let flags = PageFlags {
                writable: entry & WRITE != 0,
                executable: entry & NO_EXEC == 0,
                cache: CacheMode::WriteBack,
            };
            (PhysAddr::new(entry & mask), flags)
        }

        fn restrict(writable: bool, no_execute: bool, entry: u64) -> (bool, bool) {
            (
                writable && entry & WRITE != 0,
                no_execute || entry & NO_EXEC != 0,
            )
        }

        fn canonical(virt: u64) -> u64 {
            if virt & (1 << 47) != 0 {
                virt | 0xffff_0000_0000_0000
            } else {
                virt
            }
        }
    }

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

    /// Conta as entradas publicadas à CPU e a raiz em que ela foi ativada.
    #[derive(Default)]
    struct Cpu {
        changed: RefCell<Vec<u64>>,
        activated: RefCell<Option<u64>>,
    }

    impl MmuControl for &Cpu {
        fn entry_changed(&self, virt: VirtAddr) {
            self.changed.borrow_mut().push(virt.as_u64());
        }

        fn activate(&self, root: PhysAddr) {
            *self.activated.borrow_mut() = Some(root.as_u64());
        }
    }

    struct Frames(Vec<u64>);

    impl FrameSource for Frames {
        fn allocate_frame(&mut self) -> Option<PhysAddr> {
            self.0.pop().map(PhysAddr::new)
        }
    }

    type Space<'a> = PageTables<Toy, FakeTables, &'a Cpu>;

    const ROOT: PhysAddr = PhysAddr::new(0x1000);
    const VIRT: VirtAddr = VirtAddr::new(0xffff_ff00_0000_0000);
    const LARGE_VIRT: VirtAddr = VirtAddr::new(0xffff_8000_0020_0000);
    const DEVICE: PhysAddr = PhysAddr::new(0xFEE0_0000);

    fn space(cpu: &Cpu) -> Space<'_> {
        PageTables::new(ROOT, FakeTables::default(), cpu)
    }

    fn map(space: &mut Space<'_>, virt: VirtAddr, frames: &mut Frames) -> Result<(), MapError> {
        space.map_page(virt, DEVICE, PageFlags::DEVICE, frames)
    }

    #[test]
    fn indexes_follow_the_48_bit_split() {
        let virt = VirtAddr::new(0xffff_ff80_4020_3000);
        let indexes: Vec<_> = (0..LEVELS).map(|level| table_index(virt, level)).collect();
        assert_eq!(indexes, [511, 1, 1, 3]);
        assert_eq!(table_index(VirtAddr::new(0xffff_ff00_0000_0000), 0), 510);
    }

    #[test]
    fn creates_missing_levels_and_writes_leaf() {
        let (cpu, mut frames) = (Cpu::default(), Frames(vec![0x4000, 0x3000, 0x2000]));
        let mut space = space(&cpu);
        map(&mut space, VIRT, &mut frames).unwrap();
        assert_eq!(frames.0, [] as [u64; 0]);
        let tables = &space.memory;
        assert_eq!(
            tables.read(ROOT, table_index(VIRT, 0)),
            Toy::table_entry(PhysAddr::new(0x2000))
        );
        let leaf = tables.read(PhysAddr::new(0x4000), table_index(VIRT, 3));
        assert_eq!(leaf, Toy::page_entry(DEVICE, PageFlags::DEVICE));
    }

    #[test]
    fn new_intermediate_tables_start_zeroed() {
        let cpu = Cpu::default();
        let mut space = space(&cpu);
        space.memory.0.insert(0x2000, [u64::MAX; ENTRIES]);
        map(&mut space, VIRT, &mut Frames(vec![0x4000, 0x3000, 0x2000])).unwrap();
        let used = space.memory.0[&0x2000]
            .iter()
            .filter(|&&entry| entry != 0)
            .count();
        assert_eq!(used, 1);
    }

    #[test]
    fn reuses_existing_tables() {
        let (cpu, mut frames) = (Cpu::default(), Frames(vec![0x4000, 0x3000, 0x2000]));
        let mut space = space(&cpu);
        map(&mut space, VIRT, &mut frames).unwrap();
        let next = VirtAddr::new(VIRT.as_u64() + PAGE_SIZE);
        map(&mut space, next, &mut frames).unwrap();
        assert_eq!(space.memory.0.len(), 4);
    }

    #[test]
    fn refuses_to_overwrite_a_mapping() {
        let (cpu, mut frames) = (Cpu::default(), Frames(vec![0x4000, 0x3000, 0x2000]));
        let mut space = space(&cpu);
        map(&mut space, VIRT, &mut frames).unwrap();
        assert_eq!(
            map(&mut space, VIRT, &mut frames),
            Err(MapError::AlreadyMapped(VIRT))
        );
    }

    #[test]
    fn stops_at_huge_pages() {
        let cpu = Cpu::default();
        let mut space = space(&cpu);
        let directory = Toy::table_entry(PhysAddr::new(0x2000));
        space.memory.write(ROOT, table_index(VIRT, 0), directory);
        let huge = VALID | LARGE;
        space
            .memory
            .write(PhysAddr::new(0x2000), table_index(VIRT, 1), huge);
        let result = map(&mut space, VIRT, &mut Frames(vec![]));
        assert_eq!(result, Err(MapError::HugePageInTheWay(VIRT)));
    }

    #[test]
    fn reports_exhausted_frames() {
        let cpu = Cpu::default();
        let result = map(&mut space(&cpu), VIRT, &mut Frames(vec![]));
        assert_eq!(result, Err(MapError::OutOfFrames));
    }

    #[test]
    fn rejects_misaligned_addresses() {
        let cpu = Cpu::default();
        let odd = VirtAddr::new(VIRT.as_u64() + 1);
        let result = map(&mut space(&cpu), odd, &mut Frames(vec![]));
        assert_eq!(result, Err(MapError::Misaligned));
    }

    #[test]
    fn large_page_stops_one_level_above_the_small_page() {
        let (cpu, mut frames) = (Cpu::default(), Frames(vec![0x3000, 0x2000]));
        let mut space = space(&cpu);
        let phys = PhysAddr::new(0x4020_0000);
        let data = PageFlags::DATA;
        space
            .map_large_page(LARGE_VIRT, phys, data, &mut frames)
            .unwrap();
        assert_eq!(frames.0, [] as [u64; 0]);
        let directory = space
            .memory
            .read(PhysAddr::new(0x2000), table_index(LARGE_VIRT, 1));
        assert_eq!(directory, Toy::table_entry(PhysAddr::new(0x3000)));
        let leaf = space
            .memory
            .read(PhysAddr::new(0x3000), table_index(LARGE_VIRT, 2));
        assert_eq!(leaf, Toy::large_entry(phys, data));
    }

    #[test]
    fn large_page_refuses_unaligned_addresses_and_overlaps() {
        let (cpu, mut frames) = (Cpu::default(), Frames(vec![0x5000, 0x4000, 0x3000, 0x2000]));
        let mut space = space(&cpu);
        let phys = PhysAddr::new(0x4020_0000);
        let odd = VirtAddr::new(LARGE_VIRT.as_u64() + PAGE_SIZE);
        let data = PageFlags::DATA;
        assert_eq!(
            space.map_large_page(odd, phys, data, &mut frames),
            Err(MapError::Misaligned)
        );
        space
            .map_large_page(LARGE_VIRT, phys, data, &mut frames)
            .unwrap();
        assert_eq!(
            space.map_large_page(LARGE_VIRT, phys, data, &mut frames),
            Err(MapError::AlreadyMapped(LARGE_VIRT))
        );
        assert_eq!(
            space.map_page(LARGE_VIRT, phys, data, &mut frames),
            Err(MapError::HugePageInTheWay(LARGE_VIRT))
        );
    }

    #[test]
    fn large_page_refuses_a_directory_with_small_pages() {
        let (cpu, mut frames) = (Cpu::default(), Frames(vec![0x5000, 0x4000, 0x3000, 0x2000]));
        let mut space = space(&cpu);
        let data = PageFlags::DATA;
        space
            .map_page(LARGE_VIRT, DEVICE, data, &mut frames)
            .unwrap();
        let phys = PhysAddr::new(0x4020_0000);
        assert_eq!(
            space.map_large_page(LARGE_VIRT, phys, data, &mut frames),
            Err(MapError::AlreadyMapped(LARGE_VIRT))
        );
    }

    #[test]
    fn writable_and_executable_is_refused_before_any_table_is_touched() {
        let (cpu, mut frames) = (Cpu::default(), Frames(vec![0x5000, 0x4000, 0x3000, 0x2000]));
        let mut space = space(&cpu);
        let both = PageFlags {
            writable: true,
            executable: true,
            cache: CacheMode::WriteBack,
        };
        let phys = PhysAddr::new(0x4020_0000);
        assert_eq!(
            space.map_page(VIRT, phys, both, &mut frames),
            Err(MapError::WriteExecute(VIRT))
        );
        assert_eq!(
            space.map_large_page(LARGE_VIRT, phys, both, &mut frames),
            Err(MapError::WriteExecute(LARGE_VIRT))
        );
        assert_eq!((frames.0.len(), space.memory.0.len()), (4, 0));
        assert!(cpu.changed.borrow().is_empty());
    }

    #[test]
    fn every_new_leaf_is_published_to_the_cpu_and_activate_hands_over_the_root() {
        let (cpu, mut frames) = (Cpu::default(), Frames(vec![0x5000, 0x4000, 0x3000, 0x2000]));
        let mut space = space(&cpu);
        map(&mut space, VIRT, &mut frames).unwrap();
        let _ = map(&mut space, VIRT, &mut frames);
        assert_eq!(*cpu.changed.borrow(), [VIRT.as_u64()]);
        space.activate();
        assert_eq!(*cpu.activated.borrow(), Some(ROOT.as_u64()));
    }

    #[test]
    fn new_empty_takes_a_zeroed_root_from_the_frame_source() {
        let cpu = Cpu::default();
        let mut memory = FakeTables::default();
        memory.0.insert(0x9000, [u64::MAX; ENTRIES]);
        let space: Space<'_> =
            PageTables::new_empty(&mut Frames(vec![0x9000]), memory, &cpu).unwrap();
        assert_eq!(space.memory.0[&0x9000], [0; ENTRIES]);
        let none =
            PageTables::<Toy, _, _>::new_empty(&mut Frames(vec![]), FakeTables::default(), &cpu);
        assert_eq!(none.err(), Some(MapError::OutOfFrames));
    }

    fn collect(space: &Space<'_>) -> Vec<Mapping> {
        let mut found = Vec::new();
        space.walk(&mut |mapping| found.push(mapping));
        found
    }

    #[test]
    fn walk_reports_every_leaf_with_its_flags_in_address_order() {
        let cpu = Cpu::default();
        let mut space = space(&cpu);
        let mut frames = Frames((2..10).rev().map(|n| n * 0x1000).collect());
        let code = VirtAddr::new(0xffff_ffff_8000_0000);
        let big = PhysAddr::new(0x4020_0000);
        let code_phys = PhysAddr::new(0x20_0000);
        space
            .map_page(code, code_phys, PageFlags::CODE, &mut frames)
            .unwrap();
        space
            .map_large_page(LARGE_VIRT, big, PageFlags::DATA, &mut frames)
            .unwrap();
        let found = collect(&space);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].virt, LARGE_VIRT);
        assert_eq!((found[0].phys, found[0].size), (big, LARGE_PAGE_SIZE));
        assert_eq!(found[0].flags, PageFlags::DATA);
        assert_eq!(found[1].virt, code);
        assert_eq!((found[1].phys, found[1].size), (code_phys, PAGE_SIZE));
        assert_eq!(found[1].flags, PageFlags::CODE);
    }

    #[test]
    fn walk_applies_the_restrictions_of_the_upper_levels() {
        let cpu = Cpu::default();
        let mut space = space(&cpu);
        let tables = &mut space.memory;
        let upper = Toy::table_entry(PhysAddr::new(0x2000)) & !WRITE;
        tables.write(ROOT, 300, upper);
        tables.write(
            PhysAddr::new(0x2000),
            0,
            Toy::table_entry(PhysAddr::new(0x3000)),
        );
        tables.write(
            PhysAddr::new(0x3000),
            0,
            Toy::table_entry(PhysAddr::new(0x4000)),
        );
        tables.write(
            PhysAddr::new(0x4000),
            0,
            Toy::page_entry(DEVICE, PageFlags::DATA),
        );
        let found = collect(&space);
        assert_eq!(found.len(), 1);
        assert!(!found[0].flags.writable);
    }

    #[test]
    fn walk_of_an_empty_root_finds_nothing() {
        let cpu = Cpu::default();
        assert_eq!(collect(&space(&cpu)).len(), 0);
    }
}
