// Registration snapshot.

impl ReadinessSource {
    #[inline]
    pub fn current(&self) -> u32 {
        self.bits.load(Ordering::Acquire)
    }

    #[inline]
    pub fn ready(&self, interest: u32) -> u32 {
        self.current() & interest
    }

    #[inline]
    pub fn ticket(&self, interest: u32) -> ReadinessTicket {
        ReadinessTicket {
            wait: self.wait.ticket(),
            interest,
        }
    }
}
