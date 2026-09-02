//! Ranked SMP locks: local ownership + runtime lock-order enforcement.

use core::ops::{Deref, DerefMut};
use super::{lockdep, SpinLock, SpinLockGuard};
use lockdep::LockClass;

pub struct RankedSpinLock<T: ?Sized> {
    class: LockClass,
    inner: SpinLock<T>,
}

unsafe impl<T: ?Sized + Send> Send for RankedSpinLock<T> {}
unsafe impl<T: ?Sized + Send> Sync for RankedSpinLock<T> {}

impl<T> RankedSpinLock<T> {
    pub const fn new(class: LockClass, value: T) -> Self {
        Self { class, inner: SpinLock::new(value) }
    }
}

impl<T: ?Sized> RankedSpinLock<T> {
    #[track_caller]
    pub fn lock(&self) -> RankedSpinLockGuard<'_, T> {
        lockdep::before_acquire(self.class);
        crate::kernel::scheduler::preempt::disable();
        let guard = self.inner.lock();
        lockdep::acquired(self.class);
        RankedSpinLockGuard { class: self.class, guard: Some(guard) }
    }

    #[track_caller]
    pub fn try_lock(&self) -> Option<RankedSpinLockGuard<'_, T>> {
        lockdep::before_acquire(self.class);
        crate::kernel::scheduler::preempt::disable();
        match self.inner.try_lock() {
            Some(guard) => {
                lockdep::acquired(self.class);
                Some(RankedSpinLockGuard { class: self.class, guard: Some(guard) })
            }
            None => {
                crate::kernel::scheduler::preempt::enable();
                None
            }
        }
    }
}

pub struct RankedSpinLockGuard<'a, T: ?Sized> {
    class: LockClass,
    guard: Option<SpinLockGuard<'a, T>>,
}

impl<T: ?Sized> Deref for RankedSpinLockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T { self.guard.as_ref().expect("ranked guard released") }
}

impl<T: ?Sized> DerefMut for RankedSpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.guard.as_mut().expect("ranked guard released")
    }
}

impl<T: ?Sized> Drop for RankedSpinLockGuard<'_, T> {
    fn drop(&mut self) {
        drop(self.guard.take());
        lockdep::released(self.class);
        crate::kernel::scheduler::preempt::enable();
    }
}
