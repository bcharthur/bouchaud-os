pub mod politique;
pub mod registry;
pub mod table;

use alloc::sync::Arc;

use super::abi::types::{HandleId, ObjectKind, Result, Rights};
use super::object::Object;

pub use table::{Entry, HandleTable, MAX_HANDLES_PER_PROCESS};

#[inline]
pub fn current_pid() -> u32 {
    crate::kernel::task::current_process().pid
}

#[inline]
pub fn current_table() -> Arc<HandleTable> {
    registry::table_for(current_pid())
}

pub fn install(object: Object, rights: Rights) -> Result<HandleId> {
    current_table().insert(Arc::new(object), rights)
}

pub fn install_for(pid: u32, object: Object, rights: Rights) -> Result<HandleId> {
    registry::table_for(pid).insert(Arc::new(object), rights)
}

pub fn open_legacy(pid: u32, kind: ObjectKind) -> Result<HandleId> {
    registry::table_for(pid).insert(
        Arc::new(Object::Legacy(kind)),
        Rights::READ | Rights::WRITE | Rights::INSPECT,
    )
}
