//! Native scheduling boundary.

pub use crate::kernel::scheduler::OrdonnanceurStats;

#[inline]
pub fn current_process() -> u32 {
    crate::kernel::scheduler::current()
}

#[inline]
pub fn yield_now() {
    crate::kernel::scheduler::yield_now();
}

#[inline]
pub fn stats() -> OrdonnanceurStats {
    crate::kernel::scheduler::stats()
}

#[inline]
pub fn state() -> &'static str {
    crate::kernel::scheduler::state()
}
