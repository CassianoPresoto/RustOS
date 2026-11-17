//! `hal::PageMapper` e `hal::AddressSpace` sobre as tabelas do `CR3` ou sobre tabelas
//! novas que o kernel monta antes de ativar.

use core::arch::asm;
use core::ptr;

use rustos_hal::{
    AddressSpace, FrameSource, MapError, Mapping, PAGE_SIZE, PageFlags, PageMapper, PhysAddr,
    VirtAddr,
};

use crate::pte::{
    ENTRIES, OUTPUT_ADDRESS, PAT_TYPE_WRITE_COMBINING, PAT_WRITE_COMBINING_INDEX, pat_type,
};
use crate::sysreg::{EFER_NXE, MSR_EFER, rdmsr};
use crate::walk::{self, TableMemory};

/// `IA32_PAT`: o Limine deixa a entrada 5 em write-combining (PROTOCOL.md, *Caching*).
const MSR_PAT: u32 = 0x277;

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

impl X86PageMapper {
    /// Raiz nova e vazia, para montar antes de `activate`.
    /// # Safety
    /// Em ring 0 com paginação de 4 níveis; todo frame de `frames` é acessível em
    /// `físico + offset` antes e depois da troca. Só ativar com código, pilha e mapa direto.
    pub unsafe fn new_space<F: FrameSource>(
        frames: &mut F,
        direct_map_offset: u64,
    ) -> Result<Self, MapError> {
        // SAFETY: `EFER` existe em todo x86_64.
        if unsafe { rdmsr(MSR_EFER) } & EFER_NXE == 0 {
            return Err(MapError::Unsupported("NX is off in EFER"));
        }
        // SAFETY: o PAT existe em todo x86_64 que o Limine entrega.
        let pat = unsafe { rdmsr(MSR_PAT) };
        if pat_type(pat, PAT_WRITE_COMBINING_INDEX) != PAT_TYPE_WRITE_COMBINING {
            return Err(MapError::Unsupported("PAT entry 5 is not write-combining"));
        }
        let root = frames.allocate_frame().ok_or(MapError::OutOfFrames)?;
        let mut tables = DirectMapTables { direct_map_offset };
        tables.zero(root);
        Ok(Self { root, tables })
    }
}

impl AddressSpace for X86PageMapper {
    fn activate(&self) {
        // SAFETY: o contrato de `new_space` põe código, pilha e mapa direto no espaço;
        // a escrita no `CR3` também descarta o TLB (sem páginas globais).
        unsafe {
            asm!("mov cr3, {}", in(reg) self.root.as_u64(), options(nostack, preserves_flags));
        }
    }

    fn walk(&self, visit: &mut dyn FnMut(Mapping)) {
        walk::walk(&self.tables, self.root, visit);
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
        invalidate(virt);
        Ok(())
    }

    fn map_large_page<F: FrameSource>(
        &mut self,
        virt: VirtAddr,
        phys: PhysAddr,
        flags: PageFlags,
        frames: &mut F,
    ) -> Result<(), MapError> {
        walk::map_large_page(&mut self.tables, self.root, virt, phys, flags, frames)?;
        invalidate(virt);
        Ok(())
    }
}

fn invalidate(virt: VirtAddr) {
    // SAFETY: `invlpg` só descarta a tradução em cache; vale em ring 0.
    unsafe {
        asm!("invlpg [{}]", in(reg) virt.as_u64(), options(nostack, preserves_flags));
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
