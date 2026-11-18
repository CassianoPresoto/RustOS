//! Pedaços de paginação que precisam de `asm!` ou de ponteiro: a memória das tabelas
//! pelo mapa direto, `TTBR1_EL1`, barreiras e o `MAIR`. A caminhada é a de `core/mm`.

use core::arch::asm;
use core::ptr;

use rustos_hal::{MmuControl, PAGE_SIZE, PhysAddr, TableMemory, VirtAddr};

use crate::descriptor::{MAIR_DEVICE_INDEX, MAIR_DEVICE_NGNRNE, OUTPUT_ADDRESS};

/// `TCR_EL1.EPD0`: desliga a caminhada da metade baixa (`TTBR0_EL1`).
const TCR_EPD0: u64 = 1 << 7;
/// `SCTLR_EL1.WXN` (bit 19, `sysreg` do Linux): página gravável vira não executável, em EL1 e EL0.
const SCTLR_WXN: u64 = 1 << 19;

/// Raiz da hierarquia que o `TTBR1_EL1` aponta agora.
///
/// # Safety
/// Em EL1 com MMU ligada.
pub unsafe fn active_root() -> PhysAddr {
    let ttbr1: u64;
    // SAFETY: leitura de registrador de sistema em EL1, pelo contrato acima.
    unsafe {
        asm!("mrs {}, ttbr1_el1", out(reg) ttbr1, options(nomem, nostack, preserves_flags));
    }
    PhysAddr::new(ttbr1 & OUTPUT_ADDRESS)
}

/// Escreve o atributo de dispositivo no `MAIR_EL1`.
///
/// # Safety
/// Em EL1, e nenhum mapeamento em uso aponta para o índice `MAIR_DEVICE_INDEX`.
pub unsafe fn prepare_space() {
    let shift = MAIR_DEVICE_INDEX * 8;
    let mut mair: u64;
    // SAFETY: leitura de registrador de sistema em EL1.
    unsafe { asm!("mrs {}, mair_el1", out(reg) mair, options(nomem, nostack, preserves_flags)) };
    mair = (mair & !(0xff << shift)) | (MAIR_DEVICE_NGNRNE << shift);
    // SAFETY: troca só o índice livre, pelo contrato da função; `isb` sincroniza o contexto.
    unsafe { asm!("msr mair_el1, {}", "isb", in(reg) mair, options(nostack, preserves_flags)) };
}

/// A CPU do aarch64 para o mapeador: barreiras e `TTBR1_EL1`.
#[derive(Debug)]
pub struct Aarch64Mmu;

impl MmuControl for Aarch64Mmu {
    fn entry_changed(&self, _virt: VirtAddr) {
        // SAFETY: barreiras não têm pré-condição; publicam a entrada nova para a MMU.
        unsafe { asm!("dsb ishst", "isb", options(nostack, preserves_flags)) };
    }

    fn activate(&self, root: PhysAddr) {
        // SAFETY: invalidar o TLB antes não tem pré-condição; evita duas traduções do
        // mesmo endereço (tamanho de bloco diferente) vivas ao mesmo tempo (DEBT-0026).
        unsafe {
            asm!(
                "dsb ishst",
                "tlbi vmalle1",
                "dsb ish",
                "isb",
                options(nostack)
            );
        }
        // SAFETY: quem criou o espaço pôs nele código, pilha e mapa direto.
        unsafe {
            asm!(
                "msr ttbr1_el1, {root}",
                "isb",
                root = in(reg) root.as_u64(),
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
        let mut sctlr: u64;
        // SAFETY: leitura de registrador de sistema em EL1.
        unsafe {
            asm!("mrs {}, sctlr_el1", out(reg) sctlr, options(nomem, nostack, preserves_flags));
        }
        sctlr |= SCTLR_WXN;
        // SAFETY: as tabelas do kernel são conferidas sem W+X; ligar `WXN` não muda nenhuma
        // permissão delas, só trava o que um erro futuro criasse.
        unsafe {
            asm!("msr sctlr_el1, {}", "isb", in(reg) sctlr, options(nostack, preserves_flags))
        };
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

/// `SCTLR_EL1.WXN` está ligado nesta CPU.
pub fn wxn_active() -> bool {
    let sctlr: u64;
    // SAFETY: leitura de registrador de sistema em EL1, sem efeito colateral.
    unsafe {
        asm!("mrs {}, sctlr_el1", out(reg) sctlr, options(nomem, nostack, preserves_flags));
    }
    sctlr & SCTLR_WXN != 0
}
