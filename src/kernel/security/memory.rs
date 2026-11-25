use alloc::vec::Vec;

use crate::kernel::task;
use crate::kernel::vma::Backing;

use super::capability::Capabilities;
use super::policy::Snapshot;

pub const PROT_READ: u32 = 1;
pub const PROT_WRITE: u32 = 2;
pub const PROT_EXEC: u32 = 4;
pub const MAP_ANONYMOUS: u32 = 0x20;
pub const MAP_SHARED: u32 = 0x01;

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
    prive: bool,
) -> Result<(), MemoryDeny> {
    let write = prot & PROT_WRITE != 0;
    let execute = prot & PROT_EXEC != 0;

    // Structural W^X: even System can never own writable+executable pages --
    // except anonymous PRIVATE memory of a process holding JIT_RWX
    // (BOUCHAUD_JIT_RWX_V1), which System itself does not hold.
    if write && execute
        && !crate::kernel::security::wx::exception_jit_rwx(
            prot,
            PROT_WRITE,
            PROT_EXEC,
            anonymous,
            prive,
            security.capabilities.contains(Capabilities::JIT_RWX),
        )
    {
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
    check_protection(
        security,
        prot,
        flags & MAP_ANONYMOUS != 0,
        flags & MAP_SHARED == 0,
    )
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
    // BOUCHAUD_JIT_RWX_V1 : pour l'exception RWX, TOUTE la plage doit etre
    // anonyme. `anonymous` (au moins une VMA anonyme) suffit a la regle JIT
    // ordinaire, qui RESTREINT ; ici il s'agit d'ACCORDER, et une plage melant
    // memoire anonyme et fichier prive ne doit pas devenir RWX.
    let mut autre_que_anonyme = false;
    let mut shared_nodes: Vec<usize> = Vec::new();
    for vma in &mm.promesses {
        if vma.debut >= end || vma.fin <= addr {
            continue;
        }
        saw_mapping = true;
        if !matches!(vma.backing, Backing::Zero) {
            autre_que_anonyme = true;
        }
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

    // Pour une demande W+X, seule une plage ENTIEREMENT anonyme, decrite par
    // de vraies VMA, peut beneficier de l'exception : des metadonnees absentes
    // n'accordent rien. Pour toute autre demande, la regle d'origine.
    let ecriture_execution = prot & PROT_WRITE != 0 && prot & PROT_EXEC != 0;
    let anonyme = if ecriture_execution {
        saw_mapping && !autre_que_anonyme
    } else {
        anonymous
    };
    let prive = shared_nodes.is_empty();
    check_protection(security, prot, anonyme, prive)?;

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
