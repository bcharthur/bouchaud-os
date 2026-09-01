use super::audit;
use super::capability::Capabilities;
use super::filesystem;
use super::policy;

fn restricted_path(path: &str) -> bool {
    path == "/tmp"
        || path.starts_with("/tmp/")
        || path == "/var/tmp"
        || path.starts_with("/var/tmp/")
        || path == "/dev"
        || path.starts_with("/dev/")
        || path == "/proc"
        || path.starts_with("/proc/")
        || path == "/sys"
        || path.starts_with("/sys/")
}

pub fn authorize_node(path: &str, node: usize, cwd: usize) -> Result<(), &'static str> {
    // Kernel launch/autorun has no current ring3 task. Use the caller-provided
    // resolved cwd instead of consulting task::current_process().
    let canonical = filesystem::canonical_from_node(cwd, path);
    let security = policy::launch(canonical.as_str());

    if !security.capabilities.contains(Capabilities::EXEC) {
        audit::deny(
            security.pid,
            security.credentials.euid,
            "exec-capability",
            node as u64,
        );
        return Err("execution interdite par la politique de securite");
    }

    if restricted_path(canonical.as_str())
        && !security.capabilities.contains(Capabilities::EXEC_UNTRUSTED)
    {
        audit::deny(
            security.pid,
            security.credentials.euid,
            "exec-untrusted-path",
            node as u64,
        );
        return Err("execution depuis un emplacement non fiable interdite");
    }

    let fs = crate::fs::ramfs::fs();
    if node >= fs.nodes.len() || !fs.nodes[node].used {
        return Err("noeud executable invalide");
    }
    let file = &fs.nodes[node];

    let owner = file.uid as u32;
    let group = file.gid as u32;
    let mode = file.mode as u32;
    let creds = security.credentials;

    if !super::access::mode_allows(
        creds,
        owner,
        group,
        mode,
        super::access::AccessMask::EXECUTE,
        false,
    ) {
        drop(fs);
        audit::deny(
            security.pid,
            creds.euid,
            "exec-mode",
            mode as u64,
        );
        return Err("bit d'execution absent");
    }

    // A file modifiable by everyone cannot be a trusted executable for a
    // process that lacks explicit untrusted-exec authority.
    if mode & 0o002 != 0
        && !security.capabilities.contains(Capabilities::EXEC_UNTRUSTED)
    {
        drop(fs);
        audit::deny(
            security.pid,
            creds.euid,
            "exec-world-writable",
            mode as u64,
        );
        return Err("executable modifiable par tous interdit");
    }

    Ok(())
}

pub fn authorize_user_path(path: &str) -> Result<(), &'static str> {
    let process = crate::kernel::task::current_process();
    let cwd = process.metadata.lock().cwd;
    let canonical = filesystem::canonical_at(filesystem::AT_FDCWD, path)
        .unwrap_or_else(|| path.into());
    let node = {
        let fs = crate::fs::ramfs::fs();
        match fs.resolve(canonical.as_str(), 0) {
            Some(node) => node,
            None => return Ok(()), // preserve ENOENT from the Linux dispatcher
        }
    };
    authorize_node(canonical.as_str(), node, cwd)
}
