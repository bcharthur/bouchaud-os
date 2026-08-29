//! Native I/O vocabulary.
//!
//! These values are Bouchaud internal interests, NOT Linux POLL* numbers.

pub use crate::kernel::readiness::{
    ERROR,
    HANGUP,
    PRIORITY,
    READABLE,
    WRITABLE,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoInterest(pub u32);

impl IoInterest {
    pub const READ: Self = Self(READABLE);
    pub const WRITE: Self = Self(WRITABLE);
    pub const READ_WRITE: Self = Self(READABLE | WRITABLE);

    #[inline]
    pub const fn bits(self) -> u32 {
        self.0
    }
}
