// Ticket/generation boundary.

impl WaitSource {
    #[inline]
    pub fn ticket(&self) -> WaitSourceTicket {
        self.tickets.fetch_add(1, Ordering::Relaxed);
        WaitSourceTicket {
            queue: self.queue.ticket(),
            generation: self.generation.load(Ordering::SeqCst),
        }
    }

    #[inline]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    #[inline]
    pub fn changed(&self, ticket: WaitSourceTicket) -> bool {
        self.generation.load(Ordering::SeqCst) != ticket.generation
    }
}
