use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::kernel::native::abi::types::Rights;
use crate::kernel::native::object::Object;

pub const MAX_MESSAGE_BYTES: usize = 256 * 1024;
pub const MAX_MESSAGE_HANDLES: usize = 16;

#[derive(Clone)]
pub struct TransferredHandle {
    pub object: Arc<Object>,
    pub rights: Rights,
}

pub struct Message {
    pub bytes: Vec<u8>,
    pub handles: Vec<TransferredHandle>,
}

impl Message {
    pub fn new(bytes: Vec<u8>, handles: Vec<TransferredHandle>) -> Self {
        Self { bytes, handles }
    }
}
