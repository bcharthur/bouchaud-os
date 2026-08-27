/// Opaque identity that remains valid when another window is removed.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WindowId(u64);

static NEXT_WINDOW_ID: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(1);

impl WindowId {
    /// The single process-wide identity authority, shared by runtime windows
    /// and the policy model. IDs are monotonic and never recycled.
    pub(crate) fn allocate() -> Self {
        let raw = NEXT_WINDOW_ID.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        assert!(raw != u64::MAX, "WindowId exhausted");
        Self(raw)
    }
    pub const fn get(self) -> u64 { self.0 }
}
