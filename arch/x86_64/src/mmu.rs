//! Pedaços de paginação que precisam de `asm!` ou de ponteiro: a memória das tabelas
//! pelo mapa direto, `invlpg`, `CR3` e a conferência de NX e PAT. A caminhada é a de `core/mm`.

use core::arch::asm;
use core::ptr;

use rustos_hal::{MapError, MmuControl, PAGE_SIZE, PhysAddr, TableMemory, VirtAddr};

use crate::pte::{OUTPUT_ADDRESS, PAT_TYPE_WRITE_COMBINING, PAT_WRITE_COMBINING_INDEX, pat_type};
use crate::sysreg::{CR4_PGE, EFER_NXE, MSR_EFER, rdmsr, read_cr4};

/// `IA32_PAT`: o Limine deixa a entrada 5 em write-combining (PROTOCOL.md, *Caching*).
const MSR_PAT: u32 = 0x277;

/// Raiz da hierarquia que o `CR3` aponta agora.
///
/// # Safety
/// Em ring 0 com paginação de 4 níveis.
pub unsafe fn active_root() -> PhysAddr {
    let cr3: u64;
    // SAFETY: leitura de registrador de controle em ring 0, pelo contrato acima.
    unsafe { asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags)) };
    PhysAddr::new(cr3 & OUTPUT_ADDRESS)
}

/// Recusa o espaço novo sem NX, sem PAT 5 em write-combining ou com `CR4.PGE` ligado.
///
/// # Safety
/// Em ring 0; `EFER`, `IA32_PAT` e `CR4` existem em todo x86_64 que o Limine entrega.
pub unsafe fn require_features() -> Result<(), MapError> {
    // SAFETY: contrato desta função.
    if unsafe { rdmsr(MSR_EFER) } & EFER_NXE == 0 {
        return Err(MapError::Unsupported("NX is off in EFER"));
    }
    // SAFETY: contrato desta função.
    let pat = unsafe { rdmsr(MSR_PAT) };
    if pat_type(pat, PAT_WRITE_COMBINING_INDEX) != PAT_TYPE_WRITE_COMBINING {
        return Err(MapError::Unsupported("PAT entry 5 is not write-combining"));
    }
    if read_cr4() & CR4_PGE != 0 {
        return Err(MapError::Unsupported("CR4.PGE is on"));
    }
    Ok(())
}

/// A CPU do x86_64 para o mapeador: `invlpg` e `CR3`.
#[derive(Debug)]
pub struct X86Mmu;

impl MmuControl for X86Mmu {
    fn entry_changed(&self, virt: VirtAddr) {
        // SAFETY: `invlpg` só descarta a tradução em cache; vale em ring 0.
        unsafe {
            asm!("invlpg [{}]", in(reg) virt.as_u64(), options(nostack, preserves_flags));
        }
    }

    fn entry_removed(&self, virt: VirtAddr) {
        self.entry_changed(virt);
    }

    fn activate(&self, root: PhysAddr) {
        // SAFETY: quem criou o espaço pôs nele código, pilha e mapa direto; a escrita no
        // `CR3` descarta o TLB, sem páginas globais (`require_features`).
        unsafe {
            asm!("mov cr3, {}", in(reg) root.as_u64(), options(nostack, preserves_flags));
        }
    }
}

/// Tabelas de página lidas e escritas pelo mapa direto.
#[derive(Debug)]
pub struct DirectMapTables {
    direct_map_offset: u64,
}

impl DirectMapTables {
    /// # Safety
    /// Toda tabela e todo frame novo é acessível em `físico + direct_map_offset`, e
    /// ninguém mais mexe nas tabelas ao mesmo tempo.
    pub const unsafe fn new(direct_map_offset: u64) -> Self {
        Self { direct_map_offset }
    }

    fn entry(&self, table: PhysAddr, index: usize) -> *mut u64 {
        debug_assert!(index < (PAGE_SIZE / 8) as usize);
        let virt = table.as_u64() + self.direct_map_offset + index as u64 * 8;
        ptr::with_exposed_provenance_mut(usize::try_from(virt).expect("endereço cabe em usize"))
    }
}

impl TableMemory for DirectMapTables {
    fn read(&self, table: PhysAddr, index: usize) -> u64 {
        // SAFETY: a tabela é acessível pelo mapa direto, pelo contrato de `new`.
        unsafe { self.entry(table, index).read_volatile() }
    }

    fn write(&mut self, table: PhysAddr, index: usize, entry: u64) {
        // SAFETY: a tabela é acessível pelo mapa direto, pelo contrato de `new`.
        unsafe { self.entry(table, index).write_volatile(entry) }
    }

    fn zero(&mut self, table: PhysAddr) {
        let words = usize::try_from(PAGE_SIZE / 8).expect("página cabe em usize");
        // SAFETY: o frame novo é acessível pelo mapa direto e ainda não é usado pela MMU.
        unsafe { ptr::write_bytes(self.entry(table, 0), 0, words) }
    }
}

/// Descarta a tradução de `pages` páginas de `virt` nesta CPU; acima de 32, o TLB inteiro.
///
/// # Safety
/// Ring 0.
pub unsafe fn flush_pages(virt: u64, pages: u64) {
    if pages > FLUSH_ALL_ABOVE {
        let cr3: u64;
        // SAFETY: leitura de registrador de controle em ring 0, pelo contrato acima.
        unsafe { asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags)) };
        // SAFETY: reescrever o `CR3` com o próprio valor só esvazia o TLB.
        unsafe { asm!("mov cr3, {}", in(reg) cr3, options(nostack, preserves_flags)) };
        return;
    }
    for page in 0..pages {
        X86Mmu.entry_changed(VirtAddr::new(virt + page * PAGE_SIZE));
    }
}

/// Acima disto vale mais reescrever o `CR3` do que invalidar página a página.
const FLUSH_ALL_ABOVE: u64 = 32;
