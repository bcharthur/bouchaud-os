//! Native memory vocabulary.
//!
//! No foreign mmap flags leak into this layer.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    ReadWrite,
    ReadExecute,
    ReadWriteExecute,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sharing {
    Private,
    Shared,
}
