// Blocking side.
//
// Lost wakeup contract:
// 1. producer advances `generation` before waking;
// 2. consumer captures generation + WaitQueue ticket;
// 3. consumer rechecks generation before parking;
// 4. WaitQueue itself rechecks its ticket under the BKL before scheduler park.

impl WaitSource {
    pub fn wait(&self, ticket: WaitSourceTicket) -> WaitSourceWake {
        if self.changed(ticket) {
            self.avoided_waits.fetch_add(1, Ordering::Relaxed);
            return WaitSourceWake::AlreadyChanged;
        }

        self.waits.fetch_add(1, Ordering::Relaxed);
        self.queue.wait(ticket.queue);
        self.signaled.fetch_add(1, Ordering::Relaxed);
        WaitSourceWake::Signaled
    }

    pub fn wait_until(
        &self,
        ticket: WaitSourceTicket,
        deadline_ns: u64,
    ) -> WaitSourceWake {
        if self.changed(ticket) {
            self.avoided_waits.fetch_add(1, Ordering::Relaxed);
            return WaitSourceWake::AlreadyChanged;
        }

        self.waits.fetch_add(1, Ordering::Relaxed);
        if self.queue.wait_until(ticket.queue, deadline_ns) {
            self.signaled.fetch_add(1, Ordering::Relaxed);
            WaitSourceWake::Signaled
        } else {
            self.deadlines.fetch_add(1, Ordering::Relaxed);
            WaitSourceWake::Deadline
        }
    }
}
