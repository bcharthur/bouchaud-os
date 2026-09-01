use crate::kernel::abi::errno;
use crate::kernel::native::abi::Error as NativeError;
use crate::kernel::task;

use super::audit;
use super::capability::Capabilities;
use super::{execution, filesystem, memory, network, policy};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateDecision {
    Allow,
    Return(i64),
}

// Linux x86-64 syscall numbers used by this security boundary.
const OPEN: u64 = 2;
const MMAP: u64 = 9;
const MPROTECT: u64 = 10;
const SOCKET: u64 = 41;
const CLONE: u64 = 56;
const FORK: u64 = 57;
const VFORK: u64 = 58;
const EXECVE: u64 = 59;
const KILL: u64 = 62;
const RENAME: u64 = 82;
const MKDIR: u64 = 83;
const RMDIR: u64 = 84;
const CREAT: u64 = 85;
const LINK: u64 = 86;
const UNLINK: u64 = 87;
const SYMLINK: u64 = 88;
const CHMOD: u64 = 90;
const FCHMOD: u64 = 91;
const CHOWN: u64 = 92;
const FCHOWN: u64 = 93;
const LCHOWN: u64 = 94;
const PTRACE: u64 = 101;
const GETUID: u64 = 102;
const GETGID: u64 = 104;
const SETUID: u64 = 105;
const SETGID: u64 = 106;
const GETEUID: u64 = 107;
const GETEGID: u64 = 108;
const CAPSET: u64 = 126;
const MKNOD: u64 = 133;
const PRCTL: u64 = 157;
const CHROOT: u64 = 161;
const ACCT: u64 = 163;
const SETTIMEOFDAY: u64 = 164;
const MOUNT: u64 = 165;
const UMOUNT2: u64 = 166;
const SWAPON: u64 = 167;
const SWAPOFF: u64 = 168;
const REBOOT: u64 = 169;
const SETHOSTNAME: u64 = 170;
const SETDOMAINNAME: u64 = 171;
const IOPL: u64 = 172;
const IOPERM: u64 = 173;
const TKILL: u64 = 200;
const TGKILL: u64 = 234;
const OPENAT: u64 = 257;
const MKDIRAT: u64 = 258;
const MKNODAT: u64 = 259;
const UNLINKAT: u64 = 263;
const RENAMEAT: u64 = 264;
const RENAMEAT2: u64 = 316;

const PR_SET_NO_NEW_PRIVS: u64 = 38;
const PR_GET_NO_NEW_PRIVS: u64 = 39;

fn linux_deny(operation: &'static str, detail: u64, code: i64) -> GateDecision {
    let security = policy::current();
    audit::deny(
        security.pid,
        security.credentials.euid,
        operation,
        detail,
    );
    GateDecision::Return(-code)
}

fn require(
    wanted: Capabilities,
    operation: &'static str,
    detail: u64,
) -> Option<GateDecision> {
    let security = policy::current();
    if security.capabilities.contains(wanted) {
        None
    } else {
        audit::deny(
            security.pid,
            security.credentials.euid,
            operation,
            detail,
        );
        Some(GateDecision::Return(-errno::EPERM))
    }
}

fn path_at(pointer: u64) -> Option<alloc::string::String> {
    crate::kernel::abi::resolve_user_path(pointer)
}

fn audit_path_deny(
    security: policy::Snapshot,
    dirfd: i32,
    raw_path: &str,
    operation: &'static str,
    reason: filesystem::FsDeny,
) -> GateDecision {
    let canonical = filesystem::canonical_at(dirfd, raw_path)
        .unwrap_or_else(|| alloc::string::String::from("<invalid-dirfd>"));
    audit::deny_path(
        security.pid,
        security.credentials.euid,
        operation,
        filesystem::detail(reason),
        canonical.as_str(),
        filesystem::reason(reason),
    );
    GateDecision::Return(-errno::EACCES)
}

fn check_open(dirfd: i32, path_pointer: u64, flags: u32) -> GateDecision {
    let Some(raw_path) = path_at(path_pointer) else {
        return GateDecision::Allow;
    };
    let security = policy::current();
    match filesystem::open_allowed(security, dirfd, raw_path.as_str(), flags) {
        Ok(()) => GateDecision::Allow,
        Err(reason) => audit_path_deny(
            security, dirfd, raw_path.as_str(), "open-path", reason,
        ),
    }
}

fn check_mutation(
    dirfd: i32,
    path_pointer: u64,
    kind: filesystem::Mutation,
    operation: &'static str,
) -> GateDecision {
    let Some(raw_path) = path_at(path_pointer) else {
        return GateDecision::Allow;
    };
    let security = policy::current();
    match filesystem::mutation_allowed(security, dirfd, raw_path.as_str(), kind) {
        Ok(()) => GateDecision::Allow,
        Err(reason) => audit_path_deny(
            security, dirfd, raw_path.as_str(), operation, reason,
        ),
    }
}

fn check_reference(
    dirfd: i32,
    path_pointer: u64,
    operation: &'static str,
) -> GateDecision {
    let Some(raw_path) = path_at(path_pointer) else {
        return GateDecision::Allow;
    };
    let security = policy::current();
    match filesystem::reference_allowed(security, dirfd, raw_path.as_str()) {
        Ok(()) => GateDecision::Allow,
        Err(reason) => audit_path_deny(
            security, dirfd, raw_path.as_str(), operation, reason,
        ),
    }
}

fn check_fd_metadata(fd: i32, chown: bool, operation: &'static str) -> GateDecision {
    let security = policy::current();
    match filesystem::fd_metadata_change_allowed(security, fd, chown) {
        Ok(()) => GateDecision::Allow,
        Err(reason) => {
            audit::deny(
                security.pid,
                security.credentials.euid,
                operation,
                filesystem::detail(reason),
            );
            GateDecision::Return(-errno::EPERM)
        }
    }
}

fn signal_process_allowed(target_pid: u32, operation: &'static str) -> GateDecision {
    let security = policy::current();
    if let Some(process) = task::process_by_pid(target_pid) {
        let target_uid = process.metadata.lock().uid as u32;
        if target_uid == security.credentials.euid
            || security.capabilities.contains(Capabilities::PROCESS_CONTROL)
        {
            GateDecision::Allow
        } else {
            linux_deny(operation, target_pid as u64, errno::EPERM)
        }
    } else if security.capabilities.contains(Capabilities::PROCESS_CONTROL) {
        GateDecision::Allow
    } else {
        linux_deny(operation, target_pid as u64, errno::EPERM)
    }
}

fn signal_thread_allowed(target_tid: u32) -> GateDecision {
    let security = policy::current();
    if let Some(process) = task::process_of_tid(target_tid) {
        let target_uid = process.metadata.lock().uid as u32;
        if target_uid == security.credentials.euid
            || security.capabilities.contains(Capabilities::PROCESS_CONTROL)
        {
            GateDecision::Allow
        } else {
            linux_deny("signal-thread-other-uid", target_tid as u64, errno::EPERM)
        }
    } else if security.capabilities.contains(Capabilities::PROCESS_CONTROL) {
        GateDecision::Allow
    } else {
        linux_deny("signal-thread-unknown", target_tid as u64, errno::EPERM)
    }
}

fn gate_native(number: u64, args: [u64; 6]) -> GateDecision {
    let security = policy::current();

    if number == crate::kernel::native::abi::numbers::CHANNEL_SEND
        && args[4] != 0
        && !security.capabilities.contains(Capabilities::IPC_TRANSFER)
    {
        audit::deny(
            security.pid,
            security.credentials.euid,
            "native-handle-transfer",
            args[4],
        );
        return GateDecision::Return(NativeError::AccessDenied.neg());
    }

    if number == crate::kernel::native::abi::numbers::HANDLE_DUP
        && args[1] as u32 & crate::kernel::native::abi::Rights::TRANSFER.0 != 0
        && !security.capabilities.contains(Capabilities::IPC_TRANSFER)
    {
        audit::deny(
            security.pid,
            security.credentials.euid,
            "native-dup-transfer-right",
            args[1],
        );
        return GateDecision::Return(NativeError::AccessDenied.neg());
    }

    // This is a per-object ceiling, not cumulative accounting.  It prevents a
    // single untrusted allocation from consuming the machine while keeping the
    // native handle layer independent of the security registry.
    if number == crate::kernel::native::abi::numbers::SHM_CREATE {
        let limit = match security.profile {
            super::profile::SecurityProfile::System => 64 * 1024 * 1024u64,
            super::profile::SecurityProfile::BrowserBroker => 32 * 1024 * 1024u64,
            super::profile::SecurityProfile::BrowserContent => 16 * 1024 * 1024u64,
            super::profile::SecurityProfile::User => 16 * 1024 * 1024u64,
            super::profile::SecurityProfile::Untrusted => 4 * 1024 * 1024u64,
        };
        if args[0] > limit {
            audit::deny(
                security.pid,
                security.credentials.euid,
                "native-shm-object-limit",
                args[0],
            );
            return GateDecision::Return(NativeError::AccessDenied.neg());
        }
    }

    GateDecision::Allow
}

pub fn gate(number: u64, args: [u64; 6], native: bool) -> GateDecision {
    if native {
        return gate_native(number, args);
    }

    match number {
        MMAP => {
            let security = policy::current();
            if let Err(reason) = memory::mmap(security, args[2] as u32, args[3] as u32) {
                audit::deny(
                    security.pid,
                    security.credentials.euid,
                    "mmap-wx",
                    memory::detail(reason),
                );
                return GateDecision::Return(-errno::EACCES);
            }
            if args[3] as u32 & memory::MAP_ANONYMOUS == 0 && args[4] as i64 >= 0 {
                if let Err(reason) = filesystem::mmap_allowed(
                    security,
                    args[4] as i32,
                    args[2] as u32,
                    args[3] as u32,
                ) {
                    audit::deny(
                        security.pid,
                        security.credentials.euid,
                        "mmap-file-access",
                        filesystem::detail(reason),
                    );
                    return GateDecision::Return(-errno::EACCES);
                }
            }
            GateDecision::Allow
        }
        MPROTECT => {
            let security = policy::current();
            match memory::mprotect(security, args[0], args[1], args[2] as u32) {
                Ok(()) => GateDecision::Allow,
                Err(reason) => {
                    audit::deny(
                        security.pid,
                        security.credentials.euid,
                        "mprotect-wx",
                        memory::detail(reason),
                    );
                    GateDecision::Return(-errno::EACCES)
                }
            }
        }
        EXECVE => {
            let Some(path) = path_at(args[0]) else {
                return GateDecision::Allow;
            };
            match execution::authorize_user_path(path.as_str()) {
                Ok(()) => GateDecision::Allow,
                Err(_) => GateDecision::Return(-errno::EACCES),
            }
        }
        GETUID => GateDecision::Return(policy::current().credentials.ruid as i64),
        GETEUID => GateDecision::Return(policy::current().credentials.euid as i64),
        GETGID => GateDecision::Return(policy::current().credentials.rgid as i64),
        GETEGID => GateDecision::Return(policy::current().credentials.egid as i64),

        SETUID => match policy::set_uid_current(args[0] as u32) {
            Ok(_) => GateDecision::Return(0),
            Err(_) => linux_deny("setuid", args[0], errno::EPERM),
        },
        SETGID => match policy::set_gid_current(args[0] as u32) {
            Ok(_) => GateDecision::Return(0),
            Err(_) => linux_deny("setgid", args[0], errno::EPERM),
        },

        PRCTL if args[0] == PR_SET_NO_NEW_PRIVS => {
            if args[1] != 1 || args[2] != 0 || args[3] != 0 || args[4] != 0 {
                GateDecision::Return(-errno::EINVAL)
            } else {
                policy::set_no_new_privs_current();
                GateDecision::Return(0)
            }
        }
        PRCTL if args[0] == PR_GET_NO_NEW_PRIVS => {
            GateDecision::Return(if policy::no_new_privs_current() { 1 } else { 0 })
        }

        OPEN => check_open(filesystem::AT_FDCWD, args[0], args[1] as u32),
        OPENAT => check_open(args[0] as i32, args[1], args[2] as u32),
        CREAT => check_open(filesystem::AT_FDCWD, args[0], 0x41),

        MKDIR | MKNOD => check_mutation(
            filesystem::AT_FDCWD, args[0], filesystem::Mutation::Create, "fs-create"
        ),
        MKDIRAT | MKNODAT => check_mutation(
            args[0] as i32, args[1], filesystem::Mutation::Create, "fs-at-create"
        ),
        RMDIR | UNLINK => check_mutation(
            filesystem::AT_FDCWD, args[0], filesystem::Mutation::Remove, "fs-remove"
        ),
        UNLINKAT => check_mutation(
            args[0] as i32, args[1], filesystem::Mutation::Remove, "fs-at-remove"
        ),
        CHMOD => check_mutation(
            filesystem::AT_FDCWD, args[0], filesystem::Mutation::Chmod, "fs-chmod"
        ),
        CHOWN | LCHOWN => check_mutation(
            filesystem::AT_FDCWD, args[0], filesystem::Mutation::Chown, "fs-chown"
        ),
        FCHMOD => check_fd_metadata(args[0] as i32, false, "fs-fchmod"),
        FCHOWN => check_fd_metadata(args[0] as i32, true, "fs-fchown"),

        RENAME => {
            let first = check_mutation(
                filesystem::AT_FDCWD, args[0], filesystem::Mutation::RenameSource,
                "fs-rename-source"
            );
            if first != GateDecision::Allow {
                first
            } else {
                check_mutation(
                    filesystem::AT_FDCWD, args[1], filesystem::Mutation::RenameTarget,
                    "fs-rename-target"
                )
            }
        }
        RENAMEAT | RENAMEAT2 => {
            let first = check_mutation(
                args[0] as i32, args[1], filesystem::Mutation::RenameSource,
                "fs-renameat-source"
            );
            if first != GateDecision::Allow {
                first
            } else {
                check_mutation(
                    args[2] as i32, args[3], filesystem::Mutation::RenameTarget,
                    "fs-renameat-target"
                )
            }
        }
        LINK => {
            let first = check_reference(
                filesystem::AT_FDCWD, args[0], "fs-link-source"
            );
            if first != GateDecision::Allow {
                first
            } else {
                check_mutation(
                    filesystem::AT_FDCWD, args[1], filesystem::Mutation::LinkTarget,
                    "fs-link-target"
                )
            }
        }
        SYMLINK => check_mutation(
            filesystem::AT_FDCWD, args[1], filesystem::Mutation::SymlinkTarget,
            "fs-symlink-target"
        ),

        SOCKET => {
            let security = policy::current();
            if network::socket_allowed(security, args[0] as u32, args[1] as u32) {
                GateDecision::Allow
            } else {
                audit::deny(
                    security.pid,
                    security.credentials.euid,
                    "raw-socket",
                    (args[0] << 32) | (args[1] & 0xffff_ffff),
                );
                GateDecision::Return(-errno::EPERM)
            }
        }

        KILL => {
            let security = policy::current();
            let target = args[0] as i64;
            if target <= 0 {
                if security.capabilities.contains(Capabilities::PROCESS_CONTROL) {
                    GateDecision::Allow
                } else {
                    linux_deny("signal-process-group", args[0], errno::EPERM)
                }
            } else {
                signal_process_allowed(target as u32, "signal-other-uid")
            }
        }
        TKILL => signal_thread_allowed(args[0] as u32),
        TGKILL => signal_process_allowed(args[0] as u32, "tgkill-other-uid"),

        PTRACE => require(Capabilities::DEBUG, "ptrace", args[0])
            .unwrap_or(GateDecision::Allow),
        CAPSET => require(Capabilities::SET_IDENTITY, "capset", args[0])
            .unwrap_or(GateDecision::Allow),
        CHROOT => require(Capabilities::FS_ADMIN, "chroot", args[0])
            .unwrap_or(GateDecision::Allow),

        MOUNT | UMOUNT2 | SWAPON | SWAPOFF | REBOOT | SETHOSTNAME
        | SETDOMAINNAME | SETTIMEOFDAY | ACCT => {
            require(Capabilities::SYSTEM_ADMIN, "system-admin", number)
                .unwrap_or(GateDecision::Allow)
        }

        IOPL | IOPERM => require(Capabilities::DEVICE_IO, "device-admin", number)
            .unwrap_or(GateDecision::Allow),

        _ => GateDecision::Allow,
    }
}

pub fn after_syscall(number: u64, result: i64, native: bool) {
    if native || result <= 0 {
        return;
    }

    if matches!(number, FORK | VFORK | CLONE) {
        let parent_pid = task::current_process().pid;
        policy::inherit(parent_pid, result as u32);
    }
}
