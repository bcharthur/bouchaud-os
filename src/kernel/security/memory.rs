use alloc::vec::Vec;

use crate::kernel::task;
use crate::kernel::vma::Backing;

use super::capability::Capabilities;
use super::policy::Snapshot;

pub const PROT_READ: u32 = 1;
pub const PROT_WRITE: u32 = 2;
pub const PROT_EXEC: u32 = 4;
pub const MAP_ANONYMOUS: u32 = 0x20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryDeny {
    WriteExecute,
    ExecuteDenied,
    AnonymousExecute,
    SharedWriteDenied,
}

pub fn check_protection(
    security: Snapshot,
    prot: u32,
    anonymous: bool,
) -> Result<(), MemoryDeny> {
    let write = prot & PROT_WRITE != 0;
    let execute = prot & PROT_EXEC != 0;

    // Structural W^X: even System can never own writable+executable pages.
    if write && execute {
        return Err(MemoryDeny::WriteExecute);
    }

    if execute && !security.capabilities.contains(Capabilities::EXEC) {
        return Err(MemoryDeny::ExecuteDenied);
    }

    // Anonymous executable memory is the JIT boundary.
    if execute
        && anonymous
        && !security.capabilities.contains(Capabilities::JIT)
    {
        return Err(MemoryDeny::AnonymousExecute);
    }

    Ok(())
}

pub fn mmap(
    security: Snapshot,
    prot: u32,
    flags: u32,
) -> Result<(), MemoryDeny> {
    check_protection(security, prot, flags & MAP_ANONYMOUS != 0)
}

pub fn mprotect(
    security: Snapshot,
    addr: u64,
    length: u64,
    prot: u32,
) -> Result<(), MemoryDeny> {
    let process = task::current_process();
    let end = addr.saturating_add(length);
    let mm = process.mm.lock();

    let mut saw_mapping = false;
    let mut anonymous = false;
    let mut shared_nodes: Vec<usize> = Vec::new();
    for vma in &mm.promesses {
        if vma.debut >= end || vma.fin <= addr {
            continue;
        }
        saw_mapping = true;
        match &vma.backing {
            Backing::Zero => anonymous = true,
            Backing::SharedFile { node, .. } => {
                if !shared_nodes.contains(node) {
                    shared_nodes.push(*node);
                }
            }
            _ => {}
        }
    }

    // Legacy eager mappings without an explicit VMA are treated as anonymous,
    // so missing metadata cannot become a JIT-policy bypass.
    if !saw_mapping {
        anonymous = true;
    }
    drop(mm);

    check_protection(security, prot, anonymous)?;

    if prot & PROT_WRITE != 0 {
        for node in shared_nodes {
            if !super::filesystem::node_access_allowed(
                security,
                node,
                super::access::AccessMask::WRITE,
            ) {
                return Err(MemoryDeny::SharedWriteDenied);
            }
        }
    }

    Ok(())
}

pub fn detail(reason: MemoryDeny) -> u64 {
    match reason {
        MemoryDeny::WriteExecute => 1,
        MemoryDeny::ExecuteDenied => 2,
        MemoryDeny::AnonymousExecute => 3,
        MemoryDeny::SharedWriteDenied => 4,
    }
}
