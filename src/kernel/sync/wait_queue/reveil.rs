// Producer side.

impl WaitQueue {
    pub fn wake_one(&self) -> bool {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if self.waiters.load(Ordering::SeqCst) == 0 {
            WAITQ_WAKE_SANS_VERROU.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let _kernel = enter_bkl();
        crate::kernel::task::wake_wait_queue(self.key(), 1) != 0
    }

    pub fn wake_all(&self) -> usize {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if self.waiters.load(Ordering::SeqCst) == 0 {
            WAITQ_WAKE_SANS_VERROU.fetch_add(1, Ordering::Relaxed);
            return 0;
        }
        let _kernel = enter_bkl();
        crate::kernel::task::wake_wait_queue(self.key(), usize::MAX)
    }

    pub fn wake_all_bkl_held(&self) -> usize {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if self.waiters.load(Ordering::SeqCst) == 0 {
            WAITQ_WAKE_SANS_VERROU.fetch_add(1, Ordering::Relaxed);
            return 0;
        }
        crate::kernel::task::wake_wait_queue(self.key(), usize::MAX)
    }
}
