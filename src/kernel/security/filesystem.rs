use alloc::string::String;

use crate::fs::ramfs::{self, NodeKind};
use crate::kernel::fd::FdKind;
use crate::kernel::task;

use super::access::{self, AccessMask};
use super::chemins;
use super::capability::Capabilities;
use super::path;
use super::policy::Snapshot;
use super::profile::SecurityProfile;

pub const AT_FDCWD: i32 = -100;

const O_ACCMODE: u32 = 0x3;
const O_WRONLY: u32 = 0x1;
const O_RDWR: u32 = 0x2;
const O_CREAT: u32 = 0x40;
const O_TRUNC: u32 = 0x200;

const PROT_WRITE: u32 = 2;
const MAP_SHARED: u32 = 0x01;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsDeny {
    SandboxPath,
    PrivilegedDevice,
    ReadMode,
    WriteMode,
    ParentMutation,
    Ownership,
    StickyDirectory,
    MappingAccess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    Create,
    Remove,
    RenameSource,
    RenameTarget,
    LinkTarget,
    SymlinkTarget,
    Chmod,
    Chown,
}

pub const fn detail(reason: FsDeny) -> u64 {
    match reason {
        FsDeny::SandboxPath => 1,
        FsDeny::PrivilegedDevice => 2,
        FsDeny::ReadMode => 3,
        FsDeny::WriteMode => 4,
        FsDeny::ParentMutation => 5,
        FsDeny::Ownership => 6,
        FsDeny::StickyDirectory => 7,
        FsDeny::MappingAccess => 8,
    }
}

pub const fn reason(reason: FsDeny) -> &'static str {
    match reason {
        FsDeny::SandboxPath => "outside-sandbox",
        FsDeny::PrivilegedDevice => "privileged-device",
        FsDeny::ReadMode => "read-mode-denied",
        FsDeny::WriteMode => "write-mode-denied",
        FsDeny::ParentMutation => "parent-mutation-denied",
        FsDeny::Ownership => "ownership-denied",
        FsDeny::StickyDirectory => "sticky-directory-denied",
        FsDeny::MappingAccess => "mapping-access-denied",
    }
}

fn privileged_device(path: &str) -> bool {
    path == "/dev/fb0"
        || path.starts_with("/dev/input/")
        || path == "/dev/mem"
        || path == "/dev/port"
        || path.starts_with("/dev/ata")
        || path.starts_with("/dev/pci")
}

fn wants_write(flags: u32) -> bool {
    matches!(flags & O_ACCMODE, O_WRONLY | O_RDWR)
        || flags & (O_CREAT | O_TRUNC) != 0
}

fn wants_read(flags: u32) -> bool {
    flags & O_ACCMODE != O_WRONLY
}

fn base_node_for_dirfd(dirfd: i32) -> Option<usize> {
    let process = task::current_process();
    if dirfd == AT_FDCWD {
        return Some(process.metadata.lock().cwd);
    }
    let files = process.files.lock();
    match files.get(dirfd) {
        Some(desc) => match &desc.kind {
            // A regular file is NOT a directory base.  Accepting File here
            // made the security interpretation differ from POSIX *at(2).
            FdKind::Dir(node) => Some(*node),
            _ => None,
        },
        None => None,
    }
}

/// Canonicalise a path relative to an already-resolved directory node.
/// Unlike `canonical_at`, this does not require a current ring3 task and is
/// therefore safe for kernel-side launch/autorun paths.
pub fn canonical_from_node(base_node: usize, raw_path: &str) -> String {
    if raw_path.starts_with('/') {
        return path::normalize_absolute(raw_path);
    }

    let base = {
        let fs = ramfs::fs();
        ramfs::path_string(&fs, base_node)
    };
    path::canonical_from_base(base.as_str(), raw_path)
}

/// Resolve the lexical identity that a path-based security rule must inspect.
/// Invalid/non-directory dirfds return `None`; the real syscall then supplies
/// EBADF/ENOTDIR.  Security never guesses another base directory.
pub fn canonical_at(dirfd: i32, raw_path: &str) -> Option<String> {
    if raw_path.starts_with('/') {
        return Some(path::normalize_absolute(raw_path));
    }

    let node = base_node_for_dirfd(dirfd)?;
    Some(canonical_from_node(node, raw_path))
}

fn sandbox_path_allowed(security: Snapshot, canonical: &str, write: bool) -> bool {
    // Le predicat vient du profil lui-meme : reconstruire la liste ici ferait
    // qu'un profil ajoute plus tard serait oublie dans l'un des controles.
    if super::profile::sandboxe(security.profile) {
        // Les predicats dependent du PROFIL, et pas seulement du chemin : le
        // profil persistant du navigateur appartient a RequestServer et a lui
        // seul. Voir `chemins.rs`.
        if write {
            chemins::ecriture_permise(security.profile, canonical)
        } else {
            chemins::lecture_permise(security.profile, canonical)
        }
    } else {
        true
    }
}

pub fn node_access_allowed(
    security: Snapshot,
    node: usize,
    wanted: AccessMask,
) -> bool {
    let fs = ramfs::fs();
    if node >= fs.nodes.len() || !fs.nodes[node].used {
        return false;
    }
    let item = &fs.nodes[node];
    access::mode_allows(
        security.credentials,
        item.uid as u32,
        item.gid as u32,
        item.mode as u32,
        wanted,
        item.kind == NodeKind::Dir,
    )
}

fn parent_mutation_allowed(
    fs: &ramfs::FileSystem,
    security: Snapshot,
    canonical: &str,
    sticky_victim: bool,
) -> Result<(), FsDeny> {
    let Some((parent, _)) = fs.resolve_parent_name(canonical, 0) else {
        return Ok(()); // preserve ENOENT/ENOTDIR from the real syscall
    };
    if parent >= fs.nodes.len() || !fs.nodes[parent].used {
        return Ok(());
    }

    let parent_node = &fs.nodes[parent];
    let wanted = AccessMask::WRITE.union(AccessMask::EXECUTE);
    if !access::mode_allows(
        security.credentials,
        parent_node.uid as u32,
        parent_node.gid as u32,
        parent_node.mode as u32,
        wanted,
        true,
    ) {
        return Err(FsDeny::ParentMutation);
    }

    if sticky_victim {
        if let Some(victim) = fs.resolve(canonical, 0) {
            if victim < fs.nodes.len() && fs.nodes[victim].used {
                if !access::sticky_allows(
                    security.credentials,
                    parent_node.uid as u32,
                    parent_node.mode as u32,
                    fs.nodes[victim].uid as u32,
                ) {
                    return Err(FsDeny::StickyDirectory);
                }
            }
        }
    }
    Ok(())
}

pub fn open_allowed(
    security: Snapshot,
    dirfd: i32,
    path: &str,
    flags: u32,
) -> Result<(), FsDeny> {
    let Some(canonical) = canonical_at(dirfd, path) else {
        return Ok(()); // EBADF/ENOTDIR remains the syscall's responsibility
    };

    if privileged_device(canonical.as_str())
        && !security.capabilities.contains(Capabilities::DEVICE_IO)
    {
        return Err(FsDeny::PrivilegedDevice);
    }

    let write = wants_write(flags);
    if !sandbox_path_allowed(security, canonical.as_str(), write) {
        return Err(FsDeny::SandboxPath);
    }

    let fs = ramfs::fs();
    match fs.resolve(canonical.as_str(), 0) {
        Some(node) if node < fs.nodes.len() && fs.nodes[node].used => {
            let item = &fs.nodes[node];
            if wants_read(flags)
                && !access::mode_allows(
                    security.credentials,
                    item.uid as u32,
                    item.gid as u32,
                    item.mode as u32,
                    AccessMask::READ,
                    item.kind == NodeKind::Dir,
                )
            {
                return Err(FsDeny::ReadMode);
            }
            if write
                && !access::mode_allows(
                    security.credentials,
                    item.uid as u32,
                    item.gid as u32,
                    item.mode as u32,
                    AccessMask::WRITE,
                    item.kind == NodeKind::Dir,
                )
            {
                return Err(FsDeny::WriteMode);
            }
            Ok(())
        }
        _ if flags & O_CREAT != 0 => {
            parent_mutation_allowed(&fs, security, canonical.as_str(), false)
        }
        _ => Ok(()), // preserve ENOENT
    }
}

pub fn mutation_allowed(
    security: Snapshot,
    dirfd: i32,
    raw_path: &str,
    kind: Mutation,
) -> Result<(), FsDeny> {
    let Some(canonical) = canonical_at(dirfd, raw_path) else {
        return Ok(());
    };

    if !sandbox_path_allowed(security, canonical.as_str(), true) {
        return Err(FsDeny::SandboxPath);
    }

    let fs = ramfs::fs();
    match kind {
        Mutation::Create | Mutation::RenameTarget | Mutation::LinkTarget
        | Mutation::SymlinkTarget => {
            parent_mutation_allowed(&fs, security, canonical.as_str(), false)
        }
        Mutation::Remove | Mutation::RenameSource => {
            parent_mutation_allowed(&fs, security, canonical.as_str(), true)
        }
        Mutation::Chown => {
            if security.capabilities.contains(Capabilities::FS_ADMIN) {
                Ok(())
            } else {
                Err(FsDeny::Ownership)
            }
        }
        Mutation::Chmod => {
            let Some(node) = fs.resolve(canonical.as_str(), 0) else {
                return Ok(());
            };
            if node >= fs.nodes.len() || !fs.nodes[node].used {
                return Ok(());
            }
            if security.credentials.euid == fs.nodes[node].uid as u32
                || security.capabilities.contains(Capabilities::FS_ADMIN)
            {
                Ok(())
            } else {
                Err(FsDeny::Ownership)
            }
        }
    }
}

/// Validate the descriptor and DAC requirements of a file-backed mmap.  This
/// closes the classic `open(O_RDONLY) -> MAP_SHARED|PROT_WRITE` bypass.
pub fn mmap_allowed(
    security: Snapshot,
    fd: i32,
    prot: u32,
    flags: u32,
) -> Result<(), FsDeny> {
    let process = task::current_process();
    let (kind, open_flags) = {
        let files = process.files.lock();
        let Some(desc) = files.get(fd) else {
            return Ok(()); // preserve EBADF
        };
        (desc.kind.clone(), desc.flags)
    };

    match kind {
        FdKind::File(node) => {
            let mut wanted = AccessMask::READ;
            if flags & MAP_SHARED != 0 && prot & PROT_WRITE != 0 {
                // Shared writable mappings require an actually writable fd.
                if open_flags & O_ACCMODE != O_RDWR {
                    return Err(FsDeny::MappingAccess);
                }
                wanted = wanted.union(AccessMask::WRITE);
            } else if open_flags & O_ACCMODE == O_WRONLY {
                return Err(FsDeny::MappingAccess);
            }

            if node_access_allowed(security, node, wanted) {
                Ok(())
            } else {
                Err(FsDeny::MappingAccess)
            }
        }
        FdKind::Framebuffer => {
            if security.capabilities.contains(Capabilities::DEVICE_IO) {
                Ok(())
            } else {
                Err(FsDeny::PrivilegedDevice)
            }
        }
        _ => Ok(()),
    }
}

pub fn reference_allowed(
    security: Snapshot,
    dirfd: i32,
    raw_path: &str,
) -> Result<(), FsDeny> {
    let Some(canonical) = canonical_at(dirfd, raw_path) else {
        return Ok(());
    };
    if !sandbox_path_allowed(security, canonical.as_str(), false) {
        return Err(FsDeny::SandboxPath);
    }
    let fs = ramfs::fs();
    let Some(node) = fs.resolve(canonical.as_str(), 0) else {
        return Ok(());
    };
    if node >= fs.nodes.len() || !fs.nodes[node].used {
        return Ok(());
    }
    let item = &fs.nodes[node];
    if access::mode_allows(
        security.credentials,
        item.uid as u32,
        item.gid as u32,
        item.mode as u32,
        AccessMask::READ,
        item.kind == NodeKind::Dir,
    ) {
        Ok(())
    } else {
        Err(FsDeny::ReadMode)
    }
}

pub fn fd_metadata_change_allowed(
    security: Snapshot,
    fd: i32,
    chown: bool,
) -> Result<(), FsDeny> {
    let process = task::current_process();
    let kind = {
        let files = process.files.lock();
        let Some(desc) = files.get(fd) else {
            return Ok(()); // preserve EBADF
        };
        desc.kind.clone()
    };
    let (FdKind::File(node) | FdKind::Dir(node)) = kind else {
        return Ok(());
    };

    if chown {
        if security.capabilities.contains(Capabilities::FS_ADMIN) {
            return Ok(());
        }
        return Err(FsDeny::Ownership);
    }

    let fs = ramfs::fs();
    if node >= fs.nodes.len() || !fs.nodes[node].used {
        return Ok(());
    }
    if security.credentials.euid == fs.nodes[node].uid as u32
        || security.capabilities.contains(Capabilities::FS_ADMIN)
    {
        Ok(())
    } else {
        Err(FsDeny::Ownership)
    }
}
