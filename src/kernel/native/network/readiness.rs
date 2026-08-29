// Reusable readiness cell for native socket implementations.

pub struct SocketReadiness {
    source: ReadinessSource,
    publications: AtomicU64,
}

impl SocketReadiness {
    pub const fn new() -> Self {
        Self { source: ReadinessSource::new(WRITABLE), publications: AtomicU64::new(0) }
    }
    pub fn source(&self) -> &ReadinessSource { &self.source }
    pub fn set_readable(&self, on: bool) {
        if on { self.source.set(READABLE); } else { self.source.clear(READABLE); }
        self.publications.fetch_add(1, Ordering::Relaxed);
    }
    pub fn set_writable(&self, on: bool) {
        if on { self.source.set(WRITABLE); } else { self.source.clear(WRITABLE); }
        self.publications.fetch_add(1, Ordering::Relaxed);
    }
    pub fn hangup(&self) { self.source.set(HANGUP); self.publications.fetch_add(1, Ordering::Relaxed); }
    pub fn error(&self) { self.source.set(ERROR); self.publications.fetch_add(1, Ordering::Relaxed); }
}
