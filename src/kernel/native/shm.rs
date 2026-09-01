use alloc::vec;
use alloc::vec::Vec;

use crate::kernel::sync::SpinLock;

use super::abi::types::{Error, Result, Signals};

pub const MAX_SHARED_REGION: usize = 64 * 1024 * 1024;

/// Shared kernel region.
///
/// The object is genuinely shared between processes when its handle is passed
/// through a channel.  V1 exposes safe read/write syscalls; the MAP right is
/// already carried by the handle so a later zero-copy mmap backend can replace
/// this storage without changing the ABI.
pub struct SharedRegion {
    bytes: SpinLock<Vec<u8>>,
}

impl SharedRegion {
    pub fn new(size: usize) -> Result<Self> {
        if size == 0 || size > MAX_SHARED_REGION { return Err(Error::TooLarge); }
        Ok(Self { bytes: SpinLock::new(vec![0u8; size]) })
    }

    pub fn len(&self) -> usize { self.bytes.lock().len() }

    pub fn read(&self, offset: usize, len: usize) -> Result<Vec<u8>> {
        let bytes = self.bytes.lock();
        let end = offset.checked_add(len).ok_or(Error::InvalidArgument)?;
        if end > bytes.len() { return Err(Error::InvalidArgument); }
        Ok(bytes[offset..end].to_vec())
    }

    pub fn write(&self, offset: usize, data: &[u8]) -> Result<()> {
        let mut bytes = self.bytes.lock();
        let end = offset.checked_add(data.len()).ok_or(Error::InvalidArgument)?;
        if end > bytes.len() { return Err(Error::InvalidArgument); }
        bytes[offset..end].copy_from_slice(data);
        Ok(())
    }

    pub fn signals(&self) -> Signals { Signals::READABLE | Signals::WRITABLE }
}
