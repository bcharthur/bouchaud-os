impl SocketReadiness {
    pub fn publications(&self) -> u64 { self.publications.load(Ordering::Relaxed) }
}
