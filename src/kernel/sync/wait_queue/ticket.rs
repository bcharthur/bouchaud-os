// Lost-wakeup generation ticket.

impl WaitQueue {
    #[inline]
    pub fn ticket(&self) -> WaitTicket {
        WaitTicket(self.point.ticket())
    }
}
