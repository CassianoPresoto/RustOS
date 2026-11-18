//! Um dispositivo alcançável antes de o kernel ter memória e tabelas próprias: o `TTBR0_EL1`
//! (que o Limine deixa livre) recebe um mapa de 1:1 de 1 GiB, em duas páginas da pilha do chamador.

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use rustos_hal::{PageFlags, PhysAddr};

use crate::descriptor::{OUTPUT_ADDRESS, gib_block_descriptor, table_descriptor};
use crate::mmu::prepare_space;

/// As duas páginas de tabela, alinhadas como a MMU exige. Quem as cria as mantém vivas até
/// `release_identity_map`: a MMU as lê sem que o compilador saiba.
#[derive(Debug)]
#[repr(C, align(4096))]
pub struct EarlyTables {
    level0: [u64; 512],
    level1: [u64; 512],
}

impl EarlyTables {
    pub const fn new() -> Self {
        Self {
            level0: [0; 512],
            level1: [0; 512],
        }
    }
}

impl Default for EarlyTables {
    fn default() -> Self {
        Self::new()
    }
}

static SAVED_TTBR0: AtomicU64 = AtomicU64::new(0);

/// `TCR_EL1.T0SZ` de 4 níveis (48 bits): a caminhada começa no nível 0.
const T0SZ_FOUR_LEVELS: u64 = 16;
const T0SZ_MASK: u64 = 0x3f;
/// `TCR_EL1.EPD0`: desliga a caminhada de `TTBR0_EL1`.
const EPD0: u64 = 1 << 7;
const GIB_SHIFT: u32 = 30;
const LEVEL0_SHIFT: u32 = 39;

/// Endereço físico de um endereço virtual mapeado, pela própria MMU (`AT S1E1R`).
fn translate(virt: u64) -> Option<u64> {
    let par: u64;
    // SAFETY: `at` só consulta a tradução; o resultado vem no `PAR_EL1`, lido depois do `isb`.
    unsafe {
        asm!(
            "at s1e1r, {virt}",
            "isb",
            "mrs {par}, par_el1",
            virt = in(reg) virt,
            par = out(reg) par,
            options(nostack, preserves_flags),
        );
    }
    (par & 1 == 0).then_some(par & OUTPUT_ADDRESS)
}

/// Mapeia 1:1 o GiB de `device`; `false` sem os 4 níveis do Limine no `TCR_EL1`.
///
/// # Safety
/// Em EL1, na entrada do kernel, antes de `Vm::activate`; o `TTBR0_EL1` não tem outro dono.
pub unsafe fn identity_map_device(tables: &mut EarlyTables, device: u64) -> bool {
    let tcr: u64;
    // SAFETY: leitura de registrador de sistema em EL1, sem efeito colateral.
    unsafe { asm!("mrs {}, tcr_el1", out(reg) tcr, options(nomem, nostack, preserves_flags)) };
    if tcr & T0SZ_MASK != T0SZ_FOUR_LEVELS || tcr & EPD0 != 0 || device >> LEVEL0_SHIFT != 0 {
        return false;
    }
    let (Some(level0), Some(level1)) = (
        translate(tables.level0.as_ptr().addr() as u64),
        translate(tables.level1.as_ptr().addr() as u64),
    ) else {
        return false;
    };
    // SAFETY: o índice 7 do `MAIR_EL1` é do kernel e nenhum mapeamento o usa ainda.
    unsafe { prepare_space() };
    let slot = usize::try_from(device >> GIB_SHIFT).unwrap_or(0);
    tables.level0[0] = table_descriptor(PhysAddr::new(level1));
    tables.level1[slot] = gib_block_descriptor(
        PhysAddr::new(device >> GIB_SHIFT << GIB_SHIFT),
        PageFlags::DEVICE,
    );
    let saved: u64;
    // SAFETY: troca só a metade baixa; `dsb` publica as tabelas antes de a MMU as ler.
    unsafe {
        asm!(
            "mrs {saved}, ttbr0_el1",
            "dsb ishst",
            "msr ttbr0_el1, {root}",
            "isb",
            "tlbi vmalle1",
            "dsb ish",
            "isb",
            saved = out(reg) saved,
            root = in(reg) level0,
            options(nostack),
        );
    }
    SAVED_TTBR0.store(saved, Ordering::Relaxed);
    true
}

/// Devolve ao `TTBR0_EL1` o valor que o Limine deixou; `tables` pode ser descartada depois.
///
/// # Safety
/// Depois de `identity_map_device` devolver `true`, sem mais acesso pelo mapa de 1:1.
pub unsafe fn release_identity_map(_tables: &mut EarlyTables) {
    // SAFETY: restaura o que `identity_map_device` guardou.
    unsafe {
        asm!(
            "msr ttbr0_el1, {saved}",
            "isb",
            "tlbi vmalle1",
            "dsb ish",
            "isb",
            saved = in(reg) SAVED_TTBR0.load(Ordering::Relaxed),
            options(nostack),
        );
    }
}
