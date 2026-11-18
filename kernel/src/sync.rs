//! Spinlock que mascara interrupção na CPU local enquanto está preso: um handler que
//! pegue o mesmo lock nunca acha o dono interrompido no meio, e outra CPU gira até ele
//! soltar. Ninguém cede a CPU (`yield_now`, `sleep_until`, `park`) com um preso; o
//! `locks_held` por CPU serve à conferência do escalonador, também em `release` (SDD 15).

use core::fmt;
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicU64, Ordering};

use rustos_hal::Cpu;
use rustos_sync::{SpinLock, SpinLockGuard};

use crate::{panic, percpu, platform};

/// Soma sem a instrução atômica: só para contador que uma CPU escreve de cada vez e as outras
/// só leem. O `lock xadd` custa de 20 ciclos em metal a centenas de ns no TCG.
pub fn bump(counter: &AtomicU64, by: u64) {
    counter.store(
        counter.load(Ordering::Relaxed).wrapping_add(by),
        Ordering::Relaxed,
    );
}

/// Quantos `IrqLock`s estão presos nesta CPU.
pub fn locks_held() -> u32 {
    percpu::current().locks_held()
}

pub struct IrqLock<T> {
    inner: SpinLock<T>,
}

impl<T> IrqLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: SpinLock::new(value),
        }
    }

    /// Com a interrupção mascarada a CPU não ouve o IPI de parada: o giro confere o sinal
    /// de pânico de outro núcleo e para ali mesmo.
    pub fn lock(&self) -> IrqGuard<'_, T> {
        let were_enabled = platform::cpu().save_and_disable_interrupts();
        let guard = loop {
            if let Some(guard) = self.inner.try_lock() {
                break guard;
            }
            while self.inner.is_locked() {
                panic::stop_if_requested();
                core::hint::spin_loop();
            }
        };
        percpu::current().lock_taken();
        IrqGuard {
            guard: ManuallyDrop::new(guard),
            were_enabled,
        }
    }

    pub fn try_lock(&self) -> Option<IrqGuard<'_, T>> {
        let cpu = platform::cpu();
        let were_enabled = cpu.save_and_disable_interrupts();
        if let Some(guard) = self.inner.try_lock() {
            percpu::current().lock_taken();
            Some(IrqGuard {
                guard: ManuallyDrop::new(guard),
                were_enabled,
            })
        } else {
            cpu.restore_interrupts(were_enabled);
            None
        }
    }
}

impl<T> IrqLock<T> {
    /// Tentativas que acharam o lock preso, desde o início.
    pub fn contentions(&self) -> u32 {
        self.inner.contentions()
    }

    /// Solta o lock de um dono que não roda mais (pânico em outro núcleo).
    ///
    /// # Safety
    /// O dono está parado para sempre e nenhum guard dele será usado de novo.
    pub unsafe fn break_lock(&self) {
        // SAFETY: o contrato desta função.
        unsafe { self.inner.force_unlock() };
    }
}

impl<T> fmt::Debug for IrqLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IrqLock").finish_non_exhaustive()
    }
}

/// Solta o lock antes de religar as interrupções: na ordem inversa, uma IRQ
/// poderia chegar com o lock ainda preso e travar o handler.
pub struct IrqGuard<'a, T> {
    guard: ManuallyDrop<SpinLockGuard<'a, T>>,
    were_enabled: bool,
}

impl<T> Deref for IrqGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> DerefMut for IrqGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> Drop for IrqGuard<'_, T> {
    fn drop(&mut self) {
        // SAFETY: o guard interno é solto só aqui, uma vez, e não é usado depois.
        unsafe { ManuallyDrop::drop(&mut self.guard) };
        percpu::current().lock_released();
        platform::cpu().restore_interrupts(self.were_enabled);
    }
}
