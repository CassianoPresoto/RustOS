//! Lock que mascara interrupção na CPU local enquanto está preso: um handler que
//! pegue o mesmo lock nunca acha o dono interrompido no meio.
//! Ninguém cede a CPU (`yield_now`, `sleep_until`, `park`) com um preso; o
//! `locks_held` serve ao `debug_assert` do escalonador (SDD 15).

use core::fmt;
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicU32, Ordering};

use rustos_hal::Cpu;
use rustos_sync::{SpinLock, SpinLockGuard};

use crate::platform;

/// `IrqLock`s presos agora. Uma CPU só até o SMP: vira estado por CPU então.
static HELD: AtomicU32 = AtomicU32::new(0);

/// Quantos `IrqLock`s estão presos nesta CPU.
pub fn locks_held() -> u32 {
    HELD.load(Ordering::Relaxed)
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

    pub fn lock(&self) -> IrqGuard<'_, T> {
        let were_enabled = platform::cpu().save_and_disable_interrupts();
        let guard = ManuallyDrop::new(self.inner.lock());
        HELD.fetch_add(1, Ordering::Relaxed);
        IrqGuard {
            guard,
            were_enabled,
        }
    }

    pub fn try_lock(&self) -> Option<IrqGuard<'_, T>> {
        let cpu = platform::cpu();
        let were_enabled = cpu.save_and_disable_interrupts();
        if let Some(guard) = self.inner.try_lock() {
            HELD.fetch_add(1, Ordering::Relaxed);
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
        HELD.fetch_sub(1, Ordering::Relaxed);
        platform::cpu().restore_interrupts(self.were_enabled);
    }
}
