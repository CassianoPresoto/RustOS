//! Temporizador genérico virtual (`CNTV`) e o contador `CNTVCT_EL0` como relógio.
//! A frequência vem de `CNTFRQ_EL0` em tempo de execução; nenhuma constante.

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::time::{period_ticks, ticks_to_ns};

/// `CNTV_CTL_EL0`: `ENABLE` ligado, `IMASK` desligado.
const CTL_ENABLE: u64 = 1;

static FREQUENCY_HZ: AtomicU64 = AtomicU64::new(0);
static RELOAD_TICKS: AtomicU64 = AtomicU64::new(0);
static EPOCH_TICKS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerError {
    /// O firmware não programou `CNTFRQ_EL0`.
    NoFrequency,
    /// A taxa pedida é zero ou passa da frequência do contador.
    PeriodOutOfRange,
}

/// Lê a frequência, marca o zero do relógio e dispara o timer a `hz` Hz.
/// Devolve a frequência. A IRQ só chega depois que o GIC a habilitar.
/// # Safety
/// Em EL1 ou acima, uma vez por CPU.
pub unsafe fn start(hz: u32) -> Result<u64, TimerError> {
    let frequency: u64;
    // SAFETY: EL1+, pelo contrato da função.
    unsafe {
        asm!("mrs {}, cntfrq_el0", out(reg) frequency, options(nomem, nostack, preserves_flags));
    };
    if frequency == 0 {
        return Err(TimerError::NoFrequency);
    }
    let reload = period_ticks(frequency, hz).ok_or(TimerError::PeriodOutOfRange)?;
    FREQUENCY_HZ.store(frequency, Ordering::Relaxed);
    RELOAD_TICKS.store(reload, Ordering::Relaxed);
    let now = counter();
    EPOCH_TICKS.store(now, Ordering::Relaxed);
    write_deadline(now.wrapping_add(reload));
    // SAFETY: EL1+; o `isb` garante que o disparo vale antes de a IRQ ser destravada.
    unsafe {
        asm!("msr cntv_ctl_el0, {}", "isb", in(reg) CTL_ENABLE, options(nostack, preserves_flags));
    };
    Ok(frequency)
}

/// Programa o próximo tique um período depois do anterior (`CNTV_CVAL_EL0`
/// absoluto, sem acumular o atraso da IRQ). Se já passou, parte de agora.
/// Chamado a cada IRQ do timer; escrever o `CVAL` baixa a IRQ.
pub fn rearm() {
    let reload = RELOAD_TICKS.load(Ordering::Relaxed);
    write_deadline(crate::time::next_deadline(
        read_deadline(),
        counter(),
        reload,
    ));
}

fn read_deadline() -> u64 {
    let deadline: u64;
    // SAFETY: leitura do valor de comparação do timer virtual, sem efeito colateral.
    unsafe {
        asm!("mrs {}, cntv_cval_el0", out(reg) deadline, options(nomem, nostack, preserves_flags));
    }
    deadline
}

fn write_deadline(deadline: u64) {
    // SAFETY: só escreve o valor de comparação do timer virtual.
    unsafe { asm!("msr cntv_cval_el0, {}", in(reg) deadline, options(nostack, preserves_flags)) };
}

/// Nanossegundos desde `start`; `None` antes dele.
pub fn now_ns() -> Option<u64> {
    let elapsed = counter().wrapping_sub(EPOCH_TICKS.load(Ordering::Relaxed));
    ticks_to_ns(elapsed, FREQUENCY_HZ.load(Ordering::Relaxed))
}

fn counter() -> u64 {
    let ticks: u64;
    // SAFETY: o `isb` impede a leitura de subir antes das instruções anteriores.
    unsafe {
        asm!("isb", "mrs {}, cntvct_el0", out(reg) ticks, options(nomem, nostack, preserves_flags));
    };
    ticks
}
