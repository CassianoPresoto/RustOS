//! Temporizador genérico virtual (`CNTV`) e o contador `CNTVCT_EL0` como relógio.
//! A frequência vem de `CNTFRQ_EL0` em tempo de execução; nenhuma constante.

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::time::{ns_to_ticks, ticks_to_ns};

/// `CNTV_CTL_EL0`: `ENABLE` ligado, `IMASK` desligado.
const CTL_ENABLE: u64 = 1;

static FREQUENCY_HZ: AtomicU64 = AtomicU64::new(0);
static EPOCH_TICKS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerError {
    /// O firmware não programou `CNTFRQ_EL0`.
    NoFrequency,
}

/// Lê a frequência, marca o zero do relógio e liga o timer, desarmado: o primeiro
/// disparo é de `arm_ns`. Devolve a frequência. A IRQ só chega depois que o GIC a habilitar.
/// # Safety
/// Em EL1 ou acima, uma vez por CPU.
pub unsafe fn start() -> Result<u64, TimerError> {
    let frequency: u64;
    // SAFETY: EL1+, pelo contrato da função.
    unsafe {
        asm!("mrs {}, cntfrq_el0", out(reg) frequency, options(nomem, nostack, preserves_flags));
    };
    if frequency == 0 {
        return Err(TimerError::NoFrequency);
    }
    FREQUENCY_HZ.store(frequency, Ordering::Relaxed);
    let now = counter();
    EPOCH_TICKS.store(now, Ordering::Relaxed);
    // Desarmado: o comparador no futuro distante, antes de o timer ligar.
    write_deadline(now.wrapping_add(frequency.saturating_mul(3600)));
    // SAFETY: EL1+; o `isb` garante que o disparo vale antes de a IRQ ser destravada.
    unsafe {
        asm!("msr cntv_ctl_el0, {}", "isb", in(reg) CTL_ENABLE, options(nostack, preserves_flags));
    };
    Ok(frequency)
}

/// Dispara quando o relógio (`now_ns`) chegar a `deadline_ns`; no passado, dispara já.
/// Um disparo só: escrever o `CNTV_CVAL_EL0` absoluto também baixa a IRQ em serviço.
pub fn arm_ns(deadline_ns: u64) {
    let frequency = FREQUENCY_HZ.load(Ordering::Relaxed);
    let ticks = ns_to_ticks(deadline_ns, frequency);
    write_deadline(EPOCH_TICKS.load(Ordering::Relaxed).wrapping_add(ticks));
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

/// Nanossegundos que `ticks` do `CNTVCT_EL0` valem; zero antes de `start`.
pub fn counter_to_ns(ticks: u64) -> u64 {
    ticks_to_ns(ticks, FREQUENCY_HZ.load(Ordering::Relaxed)).unwrap_or(0)
}

/// `CNTVCT_EL0`, em ticks de `CNTFRQ_EL0`.
pub fn counter() -> u64 {
    let ticks: u64;
    // SAFETY: o `isb` impede a leitura de subir antes das instruções anteriores.
    unsafe {
        asm!("isb", "mrs {}, cntvct_el0", out(reg) ticks, options(nomem, nostack, preserves_flags));
    };
    ticks
}
