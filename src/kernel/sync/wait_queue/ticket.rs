// Lost-wakeup generation ticket.

impl WaitQueue {
    #[inline]
    pub fn ticket(&self) -> WaitTicket {
        WaitTicket(self.point.ticket())
    }

    /// Un reveil a-t-il ete signale depuis `ticket` ? Distingue une attente
    /// finie a sa borne SANS reveil d'une borne qui a croise un reveil
    /// (BOUCHAUD_VIRTIO_GPU_IRQ_V1).
    #[inline]
    pub fn signale_depuis(&self, ticket: WaitTicket) -> bool {
        self.point.ticket() != ticket.0
    }
}
