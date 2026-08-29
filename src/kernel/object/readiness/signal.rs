// State transitions and targeted source wakeup.

impl ReadinessSource {
    pub fn set(&self, bits: u32) -> usize {
        let old = self.bits.fetch_or(bits, Ordering::AcqRel);
        let changed = bits & !old;
        if changed == 0 {
            return 0;
        }
        self.changes.fetch_add(1, Ordering::Relaxed);
        let woke = self.wait.signal_all();
        self.wakeups.fetch_add(woke as u64, Ordering::Relaxed);
        woke
    }

    pub fn clear(&self, bits: u32) {
        let old = self.bits.fetch_and(!bits, Ordering::AcqRel);
        if old & bits != 0 {
            self.changes.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn replace(&self, bits: u32) -> usize {
        let old = self.bits.swap(bits, Ordering::AcqRel);
        if old == bits {
            return 0;
        }
        self.changes.fetch_add(1, Ordering::Relaxed);
        let woke = self.wait.signal_all();
        self.wakeups.fetch_add(woke as u64, Ordering::Relaxed);
        woke
    }
}
