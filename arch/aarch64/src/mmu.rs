//! `hal::PageMapper` e `hal::AddressSpace` sobre a metade alta (TTBR1_EL1): as tabelas
//! ativas, ou tabelas novas que o kernel monta antes de ativar.

use core::arch::asm;
use core::ptr;

use rustos_hal::{
    AddressSpace, FrameSource, MapError, Mapping, PAGE_SIZE, PageFlags, PageMapper, PhysAddr,
    VirtAddr,
};

use crate::descriptor::{ENTRIES, MAIR_DEVICE_INDEX, MAIR_DEVICE_NGNRNE, OUTPUT_ADDRESS};
use crate::walk::{self, TableMemory};

#[derive(Debug)]
pub struct Aarch64PageMapper {
    root: PhysAddr,
    tables: DirectMapTables,
}

impl Aarch64PageMapper {
    /// # Safety
    /// Em EL1 com MMU ligada; toda tabela atual e todo frame novo é acessível em
    /// `físico + direct_map_offset`, e ninguém mais mexe nas tabelas ao mesmo tempo.
    pub unsafe fn active(direct_map_offset: u64) -> Self {
        let ttbr1: u64;
        // SAFETY: leitura de registrador de sistema em EL1, pelo contrato acima.
        unsafe {
            asm!("mrs {}, ttbr1_el1", out(reg) ttbr1, options(nomem, nostack, preserves_flags));
        }
        // SAFETY: o índice 7 do MAIR não é usado pelo Limine nem por nenhum mapeamento ainda.
        unsafe { install_device_attribute() };
        Self {
            root: PhysAddr::new(ttbr1 & OUTPUT_ADDRESS),
            tables: DirectMapTables { direct_map_offset },
        }
    }
}

/// `TCR_EL1.EPD0`: desliga a caminhada da metade baixa (`TTBR0_EL1`).
const TCR_EPD0: u64 = 1 << 7;

impl Aarch64PageMapper {
    /// Raiz nova e vazia, para montar antes de `activate`.
    /// # Safety
    /// Em EL1 com MMU ligada; todo frame de `frames` é acessível em `físico +
    /// offset` antes e depois da troca. Só ativar com código, pilha e mapa direto.
    pub unsafe fn new_space<F: FrameSource>(
        frames: &mut F,
        direct_map_offset: u64,
    ) -> Result<Self, MapError> {
        // SAFETY: o índice 7 do MAIR não é usado pelo Limine nem por nenhum mapeamento ainda.
        unsafe { install_device_attribute() };
        let root = frames.allocate_frame().ok_or(MapError::OutOfFrames)?;
        let mut tables = DirectMapTables { direct_map_offset };
        tables.zero(root);
        Ok(Self { root, tables })
    }
}

impl AddressSpace for Aarch64PageMapper {
    fn activate(&self) {
        // SAFETY: invalidar o TLB antes não tem pré-condição; evita duas traduções do
        // mesmo endereço (tamanho de bloco diferente) vivas ao mesmo tempo.
        unsafe {
            asm!(
                "dsb ishst",
                "tlbi vmalle1",
                "dsb ish",
                "isb",
                options(nostack)
            );
        }
        // SAFETY: o contrato de `new_space` põe código, pilha e mapa direto no espaço.
        unsafe {
            asm!(
                "msr ttbr1_el1, {root}",
                "isb",
                root = in(reg) self.root.as_u64(),
                options(nostack, preserves_flags),
            );
        }
        let mut tcr: u64;
        // SAFETY: leitura de registrador de sistema em EL1.
        unsafe { asm!("mrs {}, tcr_el1", out(reg) tcr, options(nomem, nostack, preserves_flags)) };
        tcr |= TCR_EPD0;
        // SAFETY: só liga `EPD0`: a metade baixa fica sem tabela, e acesso nela vira falta.
        unsafe { asm!("msr tcr_el1, {}", "isb", in(reg) tcr, options(nostack, preserves_flags)) };
        // SAFETY: descarta as traduções do espaço antigo; sem pré-condição.
        unsafe { asm!("tlbi vmalle1", "dsb ish", "isb", options(nostack)) };
    }

    fn walk(&self, visit: &mut dyn FnMut(Mapping)) {
        walk::walk(&self.tables, self.root, visit);
    }
}

impl PageMapper for Aarch64PageMapper {
    fn map_page<F: FrameSource>(
        &mut self,
        virt: VirtAddr,
        phys: PhysAddr,
        flags: PageFlags,
        frames: &mut F,
    ) -> Result<(), MapError> {
        walk::map_page(&mut self.tables, self.root, virt, phys, flags, frames)?;
        publish();
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
        publish();
        Ok(())
    }
}

fn publish() {
    // SAFETY: barreiras não têm pré-condição; publicam a entrada nova para a MMU.
    unsafe { asm!("dsb ishst", "isb", options(nostack, preserves_flags)) };
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

/// # Safety
/// Nenhum mapeamento em uso aponta para o índice `MAIR_DEVICE_INDEX`.
unsafe fn install_device_attribute() {
    let shift = MAIR_DEVICE_INDEX * 8;
    let mut mair: u64;
    // SAFETY: leitura de registrador de sistema em EL1.
    unsafe { asm!("mrs {}, mair_el1", out(reg) mair, options(nomem, nostack, preserves_flags)) };
    mair = (mair & !(0xff << shift)) | (MAIR_DEVICE_NGNRNE << shift);
    // SAFETY: troca só o índice livre, pelo contrato da função; `isb` sincroniza o contexto.
    unsafe { asm!("msr mair_el1, {}", "isb", in(reg) mair, options(nostack, preserves_flags)) };
}
