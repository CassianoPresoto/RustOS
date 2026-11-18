//! Spinlock test-and-test-and-set: espera lendo, só disputa a linha de cache ao tentar.

use core::cell::UnsafeCell;
use core::fmt;
use core::hint::spin_loop;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Não mascara interrupções: lock que um handler também pega tem de ser o
/// `IrqLock` do kernel, ou a CPU trava girando no próprio lock.
pub struct SpinLock<T> {
    locked: AtomicBool,
    /// Tentativas que acharam o lock preso: a medida de disputa entre CPUs.
    contended: AtomicU32,
    value: UnsafeCell<T>,
}

// SAFETY: o acesso a `value` só acontece através do guard, que existe para um
// único dono por vez graças ao `locked`; mover `T` entre threads exige `T: Send`.
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            contended: AtomicU32::new(0),
            value: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        loop {
            if let Some(guard) = self.try_lock() {
                return guard;
            }
            while self.locked.load(Ordering::Relaxed) {
                spin_loop();
            }
        }
    }

    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        // Não usar `then_some`: ele cria o guard mesmo na falha, e o `Drop` solta o lock alheio.
        let acquired = self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok();
        if acquired {
            Some(SpinLockGuard { lock: self })
        } else {
            self.contended.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Alguém segura o lock agora; só serve para girar esperando, nunca para decidir.
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    /// Quantas tentativas de pegar o lock o acharam preso, desde o início.
    pub fn contentions(&self) -> u32 {
        self.contended.load(Ordering::Relaxed)
    }

    /// Solta o lock sem o guard, para o pânico tomar o que um núcleo parado segurava.
    ///
    /// # Safety
    /// O dono não roda mais e nenhum guard dele será usado de novo.
    pub unsafe fn force_unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }

    pub fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

impl<T> fmt::Debug for SpinLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpinLock")
            .field("locked", &self.locked.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> Deref for SpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: o guard só existe com o lock adquirido, então não há outro acesso.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: o guard só existe com o lock adquirido e é emprestado como `&mut`.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

impl<T: fmt::Debug> fmt::Debug for SpinLockGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::*;

    #[test]
    fn try_lock_fails_while_held() {
        let lock = SpinLock::new(1);
        let guard = lock.lock();
        assert!(lock.try_lock().is_none());
        drop(guard);
        assert!(lock.try_lock().is_some());
    }

    #[test]
    fn counts_the_attempts_that_found_the_lock_held() {
        let lock = SpinLock::new(0);
        let guard = lock.lock();
        assert!(lock.try_lock().is_none());
        assert!(lock.try_lock().is_none());
        drop(guard);
        assert_eq!(lock.contentions(), 2);
        assert!(lock.try_lock().is_some());
        assert_eq!(lock.contentions(), 2);
    }

    #[test]
    fn force_unlock_lets_the_next_taker_in() {
        let lock = SpinLock::new(0);
        core::mem::forget(lock.lock());
        assert!(lock.is_locked());
        // SAFETY: o guard esquecido nunca é usado de novo.
        unsafe { lock.force_unlock() };
        assert!(lock.try_lock().is_some());
    }

    #[test]
    fn serializes_concurrent_increments() {
        let lock = Arc::new(SpinLock::new(0u64));
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let lock = Arc::clone(&lock);
                thread::spawn(move || {
                    for _ in 0..10_000 {
                        *lock.lock() += 1;
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(Arc::into_inner(lock).unwrap().into_inner(), 40_000);
    }
}
