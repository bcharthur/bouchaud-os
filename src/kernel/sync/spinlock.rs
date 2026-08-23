//! Small SMP-safe locking primitives for the kernel.
//!
//! NG1 introduces only non-sleeping locks. Sleeping mutexes and wait queues are
//! added in the next migration step once task blocking is converted to an
//! explicit wakeup model.

use core::arch::asm;
use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::arch::x86_64::smp;

const NO_OWNER: usize = usize::MAX;

pub struct SpinLock<T: ?Sized> {
    locked: AtomicBool,
    owner_cpu: AtomicUsize,
    value: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send> Send for SpinLock<T> {}
unsafe impl<T: ?Sized + Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            owner_cpu: AtomicUsize::new(NO_OWNER),
            value: UnsafeCell::new(value),
        }
    }

    pub fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

impl<T: ?Sized> SpinLock<T> {
    #[inline]
    fn current_cpu() -> usize {
        smp::cpu_index()
    }

    #[track_caller]
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        let cpu = Self::current_cpu();

        debug_assert!(
            self.owner_cpu.load(Ordering::Relaxed) != cpu,
            "SpinLock recursive acquisition on CPU {}",
            cpu
        );

        loop {
            if self
                .locked
                .compare_exchange_weak(
                    false,
                    true,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                self.owner_cpu.store(cpu, Ordering::Relaxed);
                return SpinLockGuard { lock: self };
            }

            while self.locked.load(Ordering::Relaxed) {
                spin_loop();
            }
        }
    }

    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let cpu = Self::current_cpu();

        if self.owner_cpu.load(Ordering::Relaxed) == cpu {
            return None;
        }

        if self
            .locked
            .compare_exchange(
                false,
                true,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_err()
        {
            return None;
        }

        self.owner_cpu.store(cpu, Ordering::Relaxed);
        Some(SpinLockGuard { lock: self })
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    pub fn owner_cpu(&self) -> Option<usize> {
        let owner = self.owner_cpu.load(Ordering::Relaxed);
        if owner == NO_OWNER { None } else { Some(owner) }
    }
}

pub struct SpinLockGuard<'a, T: ?Sized> {
    lock: &'a SpinLock<T>,
}

impl<T: ?Sized> Deref for SpinLockGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lock.value.get() }
    }
}

impl<T: ?Sized> DerefMut for SpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T: ?Sized> Drop for SpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.owner_cpu.store(NO_OWNER, Ordering::Relaxed);
        self.lock.locked.store(false, Ordering::Release);
    }
}

/// Spin lock variant for data shared with interrupt handlers.
///
/// Interrupts are disabled before acquiring the inner spin lock and restored to
/// their previous state only after the lock has been released.
pub struct SpinLockIrq<T: ?Sized> {
    inner: SpinLock<T>,
}

unsafe impl<T: ?Sized + Send> Send for SpinLockIrq<T> {}
unsafe impl<T: ?Sized + Send> Sync for SpinLockIrq<T> {}

impl<T> SpinLockIrq<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: SpinLock::new(value),
        }
    }
}

impl<T: ?Sized> SpinLockIrq<T> {
    #[inline]
    fn interrupts_enabled() -> bool {
        let flags: u64;
        unsafe {
            asm!(
                "pushfq",
                "pop {}",
                out(reg) flags,
                options(nomem, preserves_flags)
            );
        }
        flags & (1 << 9) != 0
    }

    pub fn lock(&self) -> SpinLockIrqGuard<'_, T> {
        let restore_interrupts = Self::interrupts_enabled();
        unsafe { asm!("cli", options(nomem, nostack, preserves_flags)); }
        let guard = self.inner.lock();

        SpinLockIrqGuard {
            guard: Some(guard),
            restore_interrupts,
        }
    }

    pub fn try_lock(&self) -> Option<SpinLockIrqGuard<'_, T>> {
        let restore_interrupts = Self::interrupts_enabled();
        unsafe { asm!("cli", options(nomem, nostack, preserves_flags)); }

        match self.inner.try_lock() {
            Some(guard) => Some(SpinLockIrqGuard {
                guard: Some(guard),
                restore_interrupts,
            }),
            None => {
                if restore_interrupts {
                    unsafe { asm!("sti", options(nomem, nostack, preserves_flags)); }
                }
                None
            }
        }
    }

    pub fn is_locked(&self) -> bool {
        self.inner.is_locked()
    }
}

pub struct SpinLockIrqGuard<'a, T: ?Sized> {
    guard: Option<SpinLockGuard<'a, T>>,
    restore_interrupts: bool,
}

impl<T: ?Sized> Deref for SpinLockIrqGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().expect("SpinLockIrqGuard released").deref()
    }
}

impl<T: ?Sized> DerefMut for SpinLockIrqGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard
            .as_mut()
            .expect("SpinLockIrqGuard released")
            .deref_mut()
    }
}

impl<T: ?Sized> Drop for SpinLockIrqGuard<'_, T> {
    fn drop(&mut self) {
        // Release the lock before re-enabling interrupts. Otherwise an IRQ on
        // this CPU could immediately recurse into the protected structure.
        drop(self.guard.take());

        if self.restore_interrupts {
            unsafe { asm!("sti", options(nomem, nostack, preserves_flags)); }
        }
    }
}
