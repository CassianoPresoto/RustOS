//! Temporizador genérico virtual (`CNTV`) e o contador `CNTVCT_EL0` como relógio.
//! A frequência vem de `CNTFRQ_EL0` em tempo de execução; nenhuma constante.

use core::arch::asm;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::time::{TickScale, ns_to_ticks};

/// `CNTV_CTL_EL0`: `ENABLE` ligado, `IMASK` desligado.
const CTL_ENABLE: u64 = 1;

static FREQUENCY_HZ: AtomicU64 = AtomicU64::new(0);
static EPOCH_TICKS: AtomicU64 = AtomicU64::new(0);
/// `TickScale` da frequência, em dois atômicos: `now_ns` não divide.
static SCALE_MULT: AtomicU64 = AtomicU64::new(0);
static SCALE_SHIFT: AtomicU32 = AtomicU32::new(0);

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
    let (mult, shift) = TickScale::new(frequency).map_or((0, 0), TickScale::parts);
    SCALE_MULT.store(mult, Ordering::Relaxed);
    SCALE_SHIFT.store(shift, Ordering::Relaxed);
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

/// Liga o timer desta CPU, desarmado, com o relógio e a frequência de `start` na CPU de boot.
/// # Safety
/// Em EL1 ou acima, depois de `start` na CPU de boot.
pub unsafe fn start_secondary() {
    let now = counter();
    write_deadline(now.wrapping_add(FREQUENCY_HZ.load(Ordering::Relaxed).saturating_mul(3600)));
    // SAFETY: EL1+; o `isb` garante que o disparo vale antes de a IRQ ser destravada.
    unsafe {
        asm!("msr cntv_ctl_el0, {}", "isb", in(reg) CTL_ENABLE, options(nostack, preserves_flags));
    };
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

fn scale() -> TickScale {
    TickScale::from_parts(
        SCALE_MULT.load(Ordering::Relaxed),
        SCALE_SHIFT.load(Ordering::Relaxed),
    )
}

/// Nanossegundos desde `start`; `None` antes dele.
pub fn now_ns() -> Option<u64> {
    let elapsed = counter().wrapping_sub(EPOCH_TICKS.load(Ordering::Relaxed));
    let scale = scale();
    (!scale.is_zero()).then(|| scale.convert(elapsed))
}

/// `now_ns` sem o `isb`: a leitura pode sair até algumas dezenas de ns antes das instruções
/// anteriores. Serve ao relógio do escalonador, que tolera isso (SDD 15); medida pede `now_ns`.
pub fn sched_clock_ns() -> Option<u64> {
    let elapsed = counter_nosync().wrapping_sub(EPOCH_TICKS.load(Ordering::Relaxed));
    let scale = scale();
    (!scale.is_zero()).then(|| scale.convert(elapsed))
}

/// Nanossegundos que `ticks` do `CNTVCT_EL0` valem; zero antes de `start`.
pub fn counter_to_ns(ticks: u64) -> u64 {
    scale().convert(ticks)
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

/// `CNTVCT_EL0` sem o `isb`: sem ordem contra as instruções vizinhas.
fn counter_nosync() -> u64 {
    let ticks: u64;
    // SAFETY: só lê o contador virtual, legal em EL1.
    unsafe {
        asm!("mrs {}, cntvct_el0", out(reg) ticks, options(nomem, nostack, preserves_flags));
    };
    ticks
}
