//! A saída para EL0: o estado de EL0 em cada CPU (`CNTKCTL_EL1`, `TTBR0_EL1`), a primeira volta
//! ao usuário e a troca do espaço de endereçamento desta CPU (SDD 17).

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::percpu::prefix;
use rustos_hal::{TagFlush, TlbTag};

/// `CNTKCTL_EL1.EL0VCTEN` (bit 1): EL0 lê `CNTVCT_EL0`. Os outros acessos de EL0 ao contador, aos
/// timers e ao *event stream* ficam desligados, como no Linux (`arch_counter_set_user_access`).
const CNTKCTL_EL0_VIRTUAL_COUNTER: u64 = 1 << 1;
/// `TCR_EL1.EPD0`: desliga a caminhada de `TTBR0_EL1`.
const TCR_EPD0: u64 = 1 << 7;
/// `TCR_EL1.AS` (bit 36): ASID de 16 bits em `TTBR0_EL1[63:48]`.
const TCR_AS: u64 = 1 << 36;

/// A raiz vazia que a tarefa só de kernel usa no `TTBR0_EL1`.
static EMPTY_ROOT: AtomicU64 = AtomicU64::new(0);

/// Prepara EL0 nesta CPU: `CNTKCTL_EL1`, `TTBR0_EL1` na raiz vazia e `EPD0` limpo.
/// # Safety
/// EL1, mascarada, depois de `Vm::activate` nesta CPU; `empty_root` é um frame zerado eterno.
pub unsafe fn init_user(empty_root: u64, wide_asid: bool) {
    EMPTY_ROOT.store(empty_root, Ordering::Release);
    // SAFETY: só `EL0VCTEN` fica ligado; o registrador existe em todo EL1.
    unsafe {
        asm!("msr cntkctl_el1, {}", in(reg) CNTKCTL_EL0_VIRTUAL_COUNTER, options(nostack, preserves_flags));
    }
    // SAFETY: a raiz vazia não mapeia nada; o `isb` e o `tlbi` publicam a troca.
    unsafe {
        asm!(
            "msr ttbr0_el1, {root}",
            "isb",
            "tlbi vmalle1",
            "dsb ish",
            "isb",
            root = in(reg) empty_root,
            options(nostack),
        );
    }
    prefix().active_root.store(empty_root, Ordering::Relaxed);
    let mut tcr: u64;
    // SAFETY: leitura de registrador de sistema em EL1.
    unsafe { asm!("mrs {}, tcr_el1", out(reg) tcr, options(nomem, nostack, preserves_flags)) };
    tcr &= !TCR_EPD0;
    if wide_asid {
        tcr |= TCR_AS;
    } else {
        tcr &= !TCR_AS;
    }
    // SAFETY: a metade baixa tem raiz válida e vazia; limpar `EPD0` só a habilita.
    unsafe { asm!("msr tcr_el1, {}", "isb", in(reg) tcr, options(nostack, preserves_flags)) };
}

/// Largura do ASID em bits, de `ID_AA64MMFR0_EL1.ASIDBits` (`0b0010` = 16, senão 8).
pub fn asid_bits() -> u32 {
    let mmfr0: u64;
    // SAFETY: leitura de registrador de identificação em EL1.
    unsafe {
        asm!("mrs {}, id_aa64mmfr0_el1", out(reg) mmfr0, options(nomem, nostack, preserves_flags));
    }
    if (mmfr0 >> 4) & 0xF == 0b0010 { 16 } else { 8 }
}

/// Põe no `TTBR0_EL1` a raiz da tarefa com a etiqueta; sem ela, esvazia tudo a cada troca.
///
/// # Safety
/// EL1, mascarada, depois de `init_user` nesta CPU; `root` é de um espaço de usuário válido.
pub unsafe fn switch_space(root: u64, tag: TlbTag) {
    let target = if root == 0 {
        EMPTY_ROOT.load(Ordering::Acquire)
    } else {
        root
    };
    let cpu = prefix();
    match tag {
        TlbTag::Untagged => {
            if cpu.active_root.load(Ordering::Relaxed) == target {
                return;
            }
            // SAFETY: o contrato da função; o `tlbi` local descarta o que o espaço anterior deixou.
            unsafe {
                asm!(
                    "msr ttbr0_el1, {root}",
                    "isb",
                    "tlbi vmalle1",
                    "dsb ish",
                    "isb",
                    root = in(reg) target,
                    options(nostack),
                );
            }
            cpu.active_root.store(target, Ordering::Relaxed);
        }
        TlbTag::Tagged { index, flush } => {
            let asid = u64::from(index) << 48;
            let value = target | asid;
            match flush {
                TagFlush::Keep => {}
                TagFlush::Space => {
                    // SAFETY: invalidar só o TLB desta CPU não tem pré-condição; o `dsb nsh` e o `isb` depois do
                    // `tlbi` completam a invalidação antes da escrita do `TTBR0`.
                    unsafe {
                        asm!("tlbi aside1, {asid}", "dsb nsh", "isb", asid = in(reg) asid, options(nostack));
                    }
                }
                TagFlush::All => {
                    // SAFETY: o mesmo contrato, para o TLB inteiro desta CPU.
                    unsafe { asm!("tlbi vmalle1", "dsb nsh", "isb", options(nostack)) };
                }
            }
            if flush != TagFlush::Keep || cpu.active_root.load(Ordering::Relaxed) != value {
                // SAFETY: o contrato da função; a etiqueta é da geração em curso e o `isb` publica.
                unsafe {
                    asm!("msr ttbr0_el1, {root}", "isb", root = in(reg) value, options(nostack));
                }
                cpu.active_root.store(value, Ordering::Relaxed);
            }
        }
    }
}

/// Primeira volta a EL0: `SP_EL1` no topo, EL0t com `DAIF` limpo, registradores zero (`x0` e `x1`
/// levam `arg0` e `arg1`). Nunca volta.
/// # Safety
/// EL1h, mascarada. `kernel_top` é o topo da pilha desta tarefa; `entry` e `sp` são de usuário.
pub unsafe fn enter_user(entry: u64, sp: u64, arg0: u64, arg1: u64, kernel_top: u64) -> ! {
    // SAFETY: o contrato da função; `eret` para EL0t é o fim de uma exceção sintética.
    unsafe {
        asm!(
            "mov sp, {top}",
            "msr sp_el0, {sp}",
            "msr elr_el1, {entry}",
            "msr spsr_el1, xzr",
            "msr tpidr_el0, xzr",
            "msr tpidrro_el0, xzr",
            "mov x2, xzr",
            "mov x3, xzr",
            "mov x4, xzr",
            "mov x5, xzr",
            "mov x6, xzr",
            "mov x7, xzr",
            "mov x8, xzr",
            "mov x9, xzr",
            "mov x10, xzr",
            "mov x11, xzr",
            "mov x12, xzr",
            "mov x13, xzr",
            "mov x14, xzr",
            "mov x15, xzr",
            "mov x16, xzr",
            "mov x17, xzr",
            "mov x18, xzr",
            "mov x19, xzr",
            "mov x20, xzr",
            "mov x21, xzr",
            "mov x22, xzr",
            "mov x23, xzr",
            "mov x24, xzr",
            "mov x25, xzr",
            "mov x26, xzr",
            "mov x27, xzr",
            "mov x28, xzr",
            "mov x29, xzr",
            "mov x30, xzr",
            "eret",
            top = in(reg) kernel_top,
            sp = in(reg) sp,
            entry = in(reg) entry,
            in("x0") arg0,
            in("x1") arg1,
            options(noreturn),
        );
    }
}
