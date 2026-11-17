//! `hal::PageMapper` sobre as tabelas ativas, a partir do `CR3`.

use core::arch::asm;
use core::ptr;

use rustos_hal::{FrameSource, MapError, PAGE_SIZE, PageFlags, PageMapper, PhysAddr, VirtAddr};

use crate::pte::{ENTRIES, OUTPUT_ADDRESS};
use crate::walk::{self, TableMemory};

#[derive(Debug)]
pub struct X86PageMapper {
    root: PhysAddr,
    tables: DirectMapTables,
}

impl X86PageMapper {
    /// # Safety
    /// Em ring 0 com paginação de 4 níveis; toda tabela atual e todo frame novo é
    /// acessível em `físico + direct_map_offset`, e ninguém mais mexe nas tabelas.
    pub unsafe fn active(direct_map_offset: u64) -> Self {
        let cr3: u64;
        // SAFETY: leitura de registrador de controle em ring 0, pelo contrato acima.
        unsafe { asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags)) };
        Self {
            root: PhysAddr::new(cr3 & OUTPUT_ADDRESS),
            tables: DirectMapTables { direct_map_offset },
        }
    }
}

impl PageMapper for X86PageMapper {
    fn map_page<F: FrameSource>(
        &mut self,
        virt: VirtAddr,
        phys: PhysAddr,
        flags: PageFlags,
        frames: &mut F,
    ) -> Result<(), MapError> {
        walk::map_page(&mut self.tables, self.root, virt, phys, flags, frames)?;
        // SAFETY: `invlpg` só descarta a tradução em cache; vale em ring 0.
        unsafe {
            asm!("invlpg [{}]", in(reg) virt.as_u64(), options(nostack, preserves_flags));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct DirectMapTables {
    direct_map_offset: u64,
}

impl DirectMapTables {
    fn entry(&self, table: PhysAddr, index: usize) -> *mut u64 {
        debug_assert!(index < ENTRIES);
        let virt = table.as_u64() + self.direct_map_offset + index as u64 * 8;
        ptr::with_exposed_provenance_mut(usize::try_from(virt).expect("endereço cabe em usize"))
    }
}

impl TableMemory for DirectMapTables {
    fn read(&self, table: PhysAddr, index: usize) -> u64 {
        // SAFETY: a tabela é acessível pelo mapa direto, pelo contrato de `active`.
        unsafe { self.entry(table, index).read_volatile() }
    }

    fn write(&mut self, table: PhysAddr, index: usize, entry: u64) {
        // SAFETY: a tabela é acessível pelo mapa direto, pelo contrato de `active`.
        unsafe { self.entry(table, index).write_volatile(entry) }
    }

    fn zero(&mut self, table: PhysAddr) {
        let words = usize::try_from(PAGE_SIZE / 8).expect("página cabe em usize");
        // SAFETY: o frame novo é acessível pelo mapa direto e ainda não é usado pela MMU.
        unsafe { ptr::write_bytes(self.entry(table, 0), 0, words) }
    }
}
