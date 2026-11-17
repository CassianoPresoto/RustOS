//! Interface da CPU do GICv3 por registradores de sistema (`ICC_*_EL1`): liga a
//! interface, reconhece e encerra interrupções.

use core::arch::asm;

use crate::intid::{intid_from_iar, needs_end_of_interrupt};

const ICC_SRE_SRE: u64 = 1 << 0;
/// `ICC_CTLR_EL1.EOImode`: com zero, `EOIR` faz a queda de prioridade e a desativação juntas.
const ICC_CTLR_EOIMODE: u64 = 1 << 1;
/// Todas as prioridades passam pela máscara.
const PRIORITY_MASK_ALL: u64 = 0xff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuInterfaceError {
    /// `ICC_SRE_EL1.SRE` não ficou em 1: um EL mais alto proíbe o acesso por registrador.
    SystemRegistersDisabled,
}

/// Afinidade desta CPU, no formato de `GICR_TYPER[63:32]` e de `GICD_IROUTER`.
pub fn mpidr() -> u64 {
    let mpidr: u64;
    // SAFETY: leitura de registrador de identificação, sem efeito colateral.
    unsafe { asm!("mrs {}, mpidr_el1", out(reg) mpidr, options(nomem, nostack, preserves_flags)) };
    mpidr
}

/// Liga a interface: `SRE`, `EOImode` 0, prioridades liberadas, grupo 1.
/// # Safety
/// Em EL1 ou acima, com o redistribuidor desta CPU já acordado.
pub unsafe fn enable() -> Result<(), CpuInterfaceError> {
    let mut sre: u64;
    // SAFETY: EL1+, pelo contrato da função.
    unsafe { asm!("mrs {}, icc_sre_el1", out(reg) sre, options(nomem, nostack, preserves_flags)) };
    if sre & ICC_SRE_SRE == 0 {
        sre |= ICC_SRE_SRE;
        // SAFETY: EL1+; `isb` faz o novo `SRE` valer para o que vem a seguir.
        unsafe {
            asm!("msr icc_sre_el1, {}", "isb", in(reg) sre, options(nomem, nostack, preserves_flags));
        };
        // SAFETY: EL1+, pelo contrato da função.
        unsafe {
            asm!("mrs {}, icc_sre_el1", out(reg) sre, options(nomem, nostack, preserves_flags));
        };
        if sre & ICC_SRE_SRE == 0 {
            return Err(CpuInterfaceError::SystemRegistersDisabled);
        }
    }
    let mut ctlr: u64;
    // SAFETY: `SRE` está em 1, então `ICC_CTLR_EL1` é acessível.
    unsafe {
        asm!("mrs {}, icc_ctlr_el1", out(reg) ctlr, options(nomem, nostack, preserves_flags));
    };
    ctlr &= !ICC_CTLR_EOIMODE;
    // SAFETY: `SRE` está em 1; só o `EOImode` muda.
    unsafe { asm!("msr icc_ctlr_el1, {}", in(reg) ctlr, options(nomem, nostack, preserves_flags)) };
    // SAFETY: `SRE` está em 1; liberar a máscara de prioridade não atende nada ainda.
    unsafe {
        asm!("msr icc_pmr_el1, {}", in(reg) PRIORITY_MASK_ALL, options(nomem, nostack, preserves_flags));
    };
    // SAFETY: `SRE` está em 1; o grupo 1 só entrega o que o GIC já habilitou.
    unsafe {
        asm!("msr icc_igrpen1_el1, {}", "isb", in(reg) 1_u64, options(nomem, nostack, preserves_flags));
    };
    Ok(())
}

/// Reconhece a interrupção pendente de maior prioridade e devolve o INTID
/// (1023 quando não há nenhuma).
pub(crate) fn acknowledge() -> u32 {
    let iar: u64;
    // SAFETY: só roda dentro da IRQ, com a interface ligada (`enable`).
    unsafe { asm!("mrs {}, icc_iar1_el1", out(reg) iar, options(nostack, preserves_flags)) };
    intid_from_iar(iar)
}

/// Encerra a interrupção reconhecida; INTID especial não leva EOI.
pub(crate) fn end_of_interrupt(intid: u32) {
    if needs_end_of_interrupt(intid) {
        // SAFETY: `intid` veio de `acknowledge` nesta mesma IRQ.
        unsafe {
            asm!("msr icc_eoir1_el1, {}", "isb", in(reg) u64::from(intid), options(nostack, preserves_flags));
        };
    }
}
