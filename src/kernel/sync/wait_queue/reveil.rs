// Producer side.

impl WaitQueue {
    pub fn wake_one(&self) -> bool {
        self.point.signale_seul();
        if self.point.dormeurs() == 0 {
            WAITQ_WAKE_SANS_VERROU.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        // Plus de gros verrou : `wake_wait_queue` lit le registre sans verrou
        // et tranche chaque reveil par compare_exchange.
        crate::kernel::task::wake_wait_queue(self.key(), 1) != 0
    }

    pub fn wake_all(&self) -> usize {
        self.point.signale_seul();
        if self.point.dormeurs() == 0 {
            WAITQ_WAKE_SANS_VERROU.fetch_add(1, Ordering::Relaxed);
            return 0;
        }
        crate::kernel::task::wake_wait_queue(self.key(), usize::MAX)
    }

}
