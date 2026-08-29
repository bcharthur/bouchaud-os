//! Native time boundary.

#[inline]
pub fn monotonic_ns() -> u64 {
    crate::kernel::timer::monotonic_ns()
}

#[inline]
pub fn ticks() -> u64 {
    crate::kernel::timer::ticks()
}
