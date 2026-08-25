//! Task-blocking mutex for process context.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

use super::WaitQueue;

pub struct SleepMutex<T: ?Sized> {
    locked: AtomicBool,
    waiters: WaitQueue,
    value: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send> Send for SleepMutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for SleepMutex<T> {}

impl<T> SleepMutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            waiters: WaitQueue::new(),
            value: UnsafeCell::new(value),
        }
    }
}

impl<T: ?Sized> SleepMutex<T> {
    pub fn lock(&self) -> SleepMutexGuard<'_, T> {
        debug_assert!(
            crate::arch::x86_64::cpu::interrupts_enabled(),
            "SleepMutex interdit depuis IRQ"
        );
        loop {
            if self
                .locked
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return SleepMutexGuard { mutex: self };
            }
            let ticket = self.waiters.ticket();
            if self.locked.load(Ordering::Acquire) {
                self.waiters.wait(ticket);
            }
        }
    }

    pub fn try_lock(&self) -> Option<SleepMutexGuard<'_, T>> {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| SleepMutexGuard { mutex: self })
    }
}

pub struct SleepMutexGuard<'a, T: ?Sized> {
    mutex: &'a SleepMutex<T>,
}

impl<T: ?Sized> Deref for SleepMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.value.get() }
    }
}

impl<T: ?Sized> DerefMut for SleepMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.value.get() }
    }
}

impl<T: ?Sized> Drop for SleepMutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
        self.mutex.waiters.wake_one();
    }
}
