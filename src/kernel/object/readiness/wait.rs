// Wait with recheck-after-registration.
//
// This is the property a global readiness queue could not provide safely for
// multiple unrelated objects: each object's generation is independent.

impl ReadinessSource {
    pub fn wait_until(
        &self,
        ticket: ReadinessTicket,
        deadline_ns: u64,
    ) -> (u32, WaitSourceWake) {
        let before = self.ready(ticket.interest);
        if before != 0 {
            self.immediate_hits.fetch_add(1, Ordering::Relaxed);
            return (before, WaitSourceWake::AlreadyChanged);
        }

        self.waits.fetch_add(1, Ordering::Relaxed);
        let wake = self.wait.wait_until(ticket.wait, deadline_ns);

        // Mandatory recheck after wake/deadline. The caller never trusts which
        // event caused the wake; readiness is the source of truth.
        (self.ready(ticket.interest), wake)
    }
}
