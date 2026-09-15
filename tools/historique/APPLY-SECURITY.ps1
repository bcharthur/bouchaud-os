$ErrorActionPreference = "Stop"
$Root = (Get-Location).Path
$Utf8NoBom = [System.Text.UTF8Encoding]::new($false)

function Read-Lf([string]$Path) {
    $raw = [System.IO.File]::ReadAllText((Join-Path $Root $Path))
    return $raw.Replace("`r`n", "`n")
}

function Write-Lf([string]$Path, [string]$Text) {
    [System.IO.File]::WriteAllText(
        (Join-Path $Root $Path),
        $Text.Replace("`r`n", "`n"),
        $Utf8NoBom
    )
}

function Replace-Exact(
    [string]$Path,
    [string]$Old,
    [string]$New,
    [string]$Already
) {
    $text = Read-Lf $Path
    if ($Already -and $text.Contains($Already)) {
        Write-Host "deja applique: $Path"
        return
    }
    $oldLf = $Old.Replace("`r`n", "`n")
    $newLf = $New.Replace("`r`n", "`n")
    if (-not $text.Contains($oldLf)) {
        throw "Ancre inattendue dans $Path. Overlay prevu pour HEAD 178ec56; aucun patch partiel n'a ete improvise."
    }
    Write-Lf $Path ($text.Replace($oldLf, $newLf))
    Write-Host "patch: $Path"
}

$branch = (& git branch --show-current).Trim()
if ($LASTEXITCODE -ne 0) { throw "Depot Git introuvable" }
if ($branch -ne "claude/complete-12-architecture") {
    throw "Branche inattendue: $branch"
}
$head = (& git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw "HEAD Git illisible" }
if ($head -ne "178ec566d16a4eaf69e9681c2ec1d7084a7e763f") {
    throw "HEAD inattendu: $head. Overlay construit pour 178ec56; je refuse un patch approximatif."
}

Replace-Exact `
    "src/kernel/mod.rs" `
    @'
pub mod native;
'@ `
    @'
pub mod security;
pub mod native;
'@ `
    "pub mod security;"

$oldExecute = @'
#[inline]
unsafe fn execute_syscall(frame: *mut TrapFrame, native: bool) {
    crate::kernel::task::account_kernel_enter();

    if native {
        crate::kernel::native::abi::handle(&mut *frame);

        // The Linux dispatcher normally owns this common ring3 tail. Native
        // syscalls bypass that dispatcher, so perform the two architecture-wide
        // actions here without making the native object model depend on Linux.
        if crate::kernel::task::take_need_resched() {
            crate::kernel::task::yield_now();
        }
        crate::kernel::abi::proc::deliver_pending(&mut *frame);
    } else {
        crate::kernel::abi::handle(&mut *frame);
    }

    crate::kernel::task::account_kernel_exit();
    crate::kernel::task::retire_current_if_zombie();
}
'@

$newExecute = @'
#[inline]
unsafe fn execute_syscall(frame: *mut TrapFrame, native: bool) {
    crate::kernel::task::account_kernel_enter();

    // BOUCHAUD_SECURITY_V1
    // Security is a mandatory boundary in front of BOTH ABIs. It can reject or
    // fully handle an operation, but it never bypasses the common ring3 tail.
    let (number, args) = (*frame).syscall_args();
    let decision = crate::kernel::security::syscall::gate(number, args, native);

    match decision {
        crate::kernel::security::syscall::GateDecision::Allow => {
            if native {
                crate::kernel::native::abi::handle(&mut *frame);

                // The Linux dispatcher normally owns this common ring3 tail.
                if crate::kernel::task::take_need_resched() {
                    crate::kernel::task::yield_now();
                }
                crate::kernel::abi::proc::deliver_pending(&mut *frame);
            } else {
                crate::kernel::abi::handle(&mut *frame);
            }
        }
        crate::kernel::security::syscall::GateDecision::Return(result) => {
            (*frame).rax = result as u64;
            if crate::kernel::task::take_need_resched() {
                crate::kernel::task::yield_now();
            }
            crate::kernel::abi::proc::deliver_pending(&mut *frame);
        }
    }

    let result = (*frame).rax as i64;
    crate::kernel::security::syscall::after_syscall(number, result, native);

    crate::kernel::task::account_kernel_exit();
    crate::kernel::task::retire_current_if_zombie();
}
'@

Replace-Exact `
    "src/arch/x86_64/usermode.rs" `
    $oldExecute `
    $newExecute `
    "BOUCHAUD_SECURITY_V1"

$oldResolve = @'
fn resolve_file_node(path: &str, cwd: usize) -> Result<usize, String> {
    let fs = crate::fs::ramfs::fs();
    match fs.resolve(path, cwd) {
        Some(node) if fs.nodes[node].kind == crate::fs::ramfs::NodeKind::File => Ok(node),
        Some(_) => Err(alloc::format!("{} : est un repertoire", path)),
        None => Err(alloc::format!("{} : fichier introuvable", path)),
    }
}
'@

$newResolve = @'
fn resolve_file_node(path: &str, cwd: usize) -> Result<usize, String> {
    let node = {
        let fs = crate::fs::ramfs::fs();
        match fs.resolve(path, cwd) {
            Some(node) if fs.nodes[node].kind == crate::fs::ramfs::NodeKind::File => node,
            Some(_) => return Err(alloc::format!("{} : est un repertoire", path)),
            None => return Err(alloc::format!("{} : fichier introuvable", path)),
        }
    };

    crate::kernel::security::execution::authorize_node(path, node, cwd)
        .map_err(|reason| alloc::format!("{} : {}", path, reason))?;

    Ok(node)
}
'@

Replace-Exact `
    "src/kernel/process/exec.rs" `
    $oldResolve `
    $newResolve `
    "security::execution::authorize_node"


# 4. execve stores a canonical image name.  Without this, `tmp/app` and
# `/tmp/app` could land in different security profiles after the exec.
Replace-Exact `
    "src/compat/linux/proc.rs" `
    @'
    let path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => path,
        None => return -errno::EFAULT,
    };
'@ `
    @'
    let path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => crate::kernel::security::filesystem::canonical_at(
            crate::kernel::security::filesystem::AT_FDCWD,
            path.as_str(),
        ).unwrap_or(path),
        None => return -errno::EFAULT,
    };
'@ `
    "security::filesystem::canonical_at"

# 5. Security state follows the Process lifetime instead of waiting for a
# periodic sweep.  This also makes future PID reuse fail closed.
Replace-Exact `
    "src/kernel/process/thread/processus.rs" `
    @'
    fn drop(&mut self) {
        let (pml4, clean, shared) = {
'@ `
    @'
    fn drop(&mut self) {
        crate::kernel::security::policy::forget(self.pid);
        let (pml4, clean, shared) = {
'@ `
    "security::policy::forget(self.pid)"

# 6. /tmp has Unix sticky semantics.  The security gate enforces ownership on
# remove/rename, so one user cannot delete another user's temporary file.
Replace-Exact `
    "src/fs/ramfs.rs" `
    @'
        if tmp != 0 {
            self.nodes[tmp].mode = 0o777;
        }
'@ `
    @'
        if tmp != 0 {
            self.nodes[tmp].mode = 0o1777;
        }
'@ `
    "self.nodes[tmp].mode = 0o1777;"

# 7. *at(2) backend semantics must match the security gate.  The historical
# mkdirat/unlinkat dispatcher discarded dirfd, and openat accepted a regular
# file as a directory base.  That mismatch is a security bug: policy could
# authorize /tmp/x while the backend actually mutated cwd/x.
Replace-Exact `
    "src/compat/linux/file.rs" `
    @'
fn resolve(path: &str) -> Option<usize> {
    let cwd = task::current_process().metadata.lock().cwd;
    ramfs::fs().resolve(path, cwd)
}
'@ `
    @'
fn resolve(path: &str) -> Option<usize> {
    let cwd = task::current_process().metadata.lock().cwd;
    ramfs::fs().resolve(path, cwd)
}

/// Construit la cible absolue d'un appel *at(2) a partir de l'identite du
/// repertoire capturee sous le verrou de la table de fichiers.
///
/// SECURITY_DIRFD_BACKEND_V21 : un fichier ordinaire n'est jamais une base de
/// chemin.  Une fois le noeud copie, fermer/reutiliser le fd dans un autre thread
/// ne change plus la cible de CET appel.
fn absolute_at(dirfd: i32, path: &str) -> Result<String, i64> {
    if path.starts_with('/') {
        return Ok(crate::kernel::security::path::normalize_absolute(path));
    }

    let base_node = if dirfd == AT_FDCWD {
        task::current_process().metadata.lock().cwd
    } else {
        let process = task::current_process();
        let files = process.files.lock();
        let Some(desc) = files.get(dirfd) else {
            return Err(-errno::EBADF);
        };
        match desc.kind {
            FdKind::Dir(node) => node,
            _ => return Err(-errno::ENOTDIR),
        }
    };

    let base = ramfs::path_string(&ramfs::fs(), base_node);
    Ok(crate::kernel::security::path::canonical_from_base(
        base.as_str(),
        path,
    ))
}

/// Recheck the RESOLVED backend target, not only the raw syscall arguments.
/// This closes the dirfd close/reuse window between the architecture gate and
/// the filesystem backend: if another thread changes the fd, the second check
/// sees the path this syscall will actually use.
fn security_recheck_open(path: &str, flags: u32) -> Result<(), i64> {
    let security = crate::kernel::security::policy::current();
    match crate::kernel::security::filesystem::open_allowed(
        security, AT_FDCWD, path, flags,
    ) {
        Ok(()) => Ok(()),
        Err(reason) => {
            crate::kernel::security::audit::deny_path(
                security.pid,
                security.credentials.euid,
                "open-path-backend",
                crate::kernel::security::filesystem::detail(reason),
                path,
                crate::kernel::security::filesystem::reason(reason),
            );
            Err(-errno::EACCES)
        }
    }
}

fn security_recheck_mutation(
    path: &str,
    kind: crate::kernel::security::filesystem::Mutation,
    operation: &'static str,
) -> Result<(), i64> {
    let security = crate::kernel::security::policy::current();
    match crate::kernel::security::filesystem::mutation_allowed(
        security, AT_FDCWD, path, kind,
    ) {
        Ok(()) => Ok(()),
        Err(reason) => {
            crate::kernel::security::audit::deny_path(
                security.pid,
                security.credentials.euid,
                operation,
                crate::kernel::security::filesystem::detail(reason),
                path,
                crate::kernel::security::filesystem::reason(reason),
            );
            Err(-errno::EACCES)
        }
    }
}
'@ `
    "SECURITY_DIRFD_BACKEND_V21"

Replace-Exact `
    "src/compat/linux/file.rs" `
    @'
pub fn sys_openat(dirfd: i32, path_addr: u64, flags: u32, mode: u32) -> i64 {
    let path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => path,
        None => return -errno::EFAULT,
    };
    let path = if dirfd != AT_FDCWD && !path.starts_with('/') {
        // Chemin relatif a un descripteur de repertoire ouvert.
        let process = task::current_process();
        let node = match process.files.lock().get(dirfd) {
            Some(desc) => match desc.kind {
                FdKind::Dir(node) | FdKind::File(node) => Some(node),
                _ => None,
            },
            None => return -errno::EBADF,
        };
        match node {
            Some(node) => {
                let mut base = ramfs::path_string(&ramfs::fs(), node);
                if !base.ends_with('/') {
                    base.push('/');
                }
                base.push_str(&path);
                base
            }
            None => absolute(&path),
        }
    } else {
        absolute(&path)
    };
'@ `
    @'
pub fn sys_openat(dirfd: i32, path_addr: u64, flags: u32, mode: u32) -> i64 {
    let raw_path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => path,
        None => return -errno::EFAULT,
    };
    let path = match absolute_at(dirfd, raw_path.as_str()) {
        Ok(path) => path,
        Err(code) => return code,
    };
    if let Err(code) = security_recheck_open(path.as_str(), flags) {
        return code;
    }
'@ `
    "security_recheck_open(path.as_str(), flags)"

Replace-Exact `
    "src/compat/linux/file.rs" `
    @'
/// `mkdir` / `mkdirat`.
pub fn sys_mkdir(path_addr: u64) -> i64 {
    let path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => absolute(&path),
        None => return -errno::EFAULT,
    };
    let cwd = task::current_process().metadata.lock().cwd;
    let mut fs = ramfs::fs();
    let (parent, name) = match fs.resolve_parent_name(&path, cwd) {
        Some(value) => value,
        None => return -errno::ENOENT,
    };
    if fs.find_child(parent, name).is_some() {
        return -errno::EEXIST;
    }
    match fs.mkdir_at(parent, name) {
        Ok(_) => 0,
        Err(raison) => -errno_creation(raison),
    }
}

/// `unlink` / `unlinkat`.
pub fn sys_unlink(path_addr: u64) -> i64 {
    let path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => absolute(&path),
        None => return -errno::EFAULT,
    };
    match resolve(&path) {
        Some(node) if node != 0 => {
            if backing::is_disk_backed(node) {
                return -errno::EROFS;
            }
            let mut fs = ramfs::fs();
            if fs.nodes[node].kind == NodeKind::Dir && !fs.is_empty_dir(node) {
                return -errno::ENOTEMPTY;
            }
            fs.nodes[node].used = false;
            fs.nodes[node].content = Vec::new();
            0
        }
        Some(_) => -errno::EBUSY,
        None => -errno::ENOENT,
    }
}
'@ `
    @'
/// `mkdir` / `mkdirat`.
pub fn sys_mkdir(path_addr: u64, mode: u32) -> i64 {
    sys_mkdirat(AT_FDCWD, path_addr, mode)
}

pub fn sys_mkdirat(dirfd: i32, path_addr: u64, mode: u32) -> i64 {
    let raw_path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => path,
        None => return -errno::EFAULT,
    };
    let path = match absolute_at(dirfd, raw_path.as_str()) {
        Ok(path) => path,
        Err(code) => return code,
    };
    if let Err(code) = security_recheck_mutation(
        path.as_str(),
        crate::kernel::security::filesystem::Mutation::Create,
        "fs-at-create-backend",
    ) {
        return code;
    }

    let mut fs = ramfs::fs();
    let (parent, name) = match fs.resolve_parent_name(&path, 0) {
        Some(value) => value,
        None => return -errno::ENOENT,
    };
    if fs.find_child(parent, name).is_some() {
        return -errno::EEXIST;
    }
    match fs.mkdir_at(parent, name) {
        Ok(node) => {
            // SECURITY_OWNER_MKDIR
            let (owner_uid, owner_gid) =
                crate::kernel::security::policy::filesystem_owner();
            fs.nodes[node].mode = (mode & 0o777) as u16;
            fs.nodes[node].uid = owner_uid;
            fs.nodes[node].gid = owner_gid;
            0
        }
        Err(raison) => -errno_creation(raison),
    }
}

/// `unlink` / `unlinkat`.
pub fn sys_unlink(path_addr: u64) -> i64 {
    sys_unlinkat(AT_FDCWD, path_addr, 0)
}

pub fn sys_unlinkat(dirfd: i32, path_addr: u64, _flags: u32) -> i64 {
    let raw_path = match crate::kernel::abi::resolve_user_path(path_addr) {
        Some(path) => path,
        None => return -errno::EFAULT,
    };
    let path = match absolute_at(dirfd, raw_path.as_str()) {
        Ok(path) => path,
        Err(code) => return code,
    };
    if let Err(code) = security_recheck_mutation(
        path.as_str(),
        crate::kernel::security::filesystem::Mutation::Remove,
        "fs-at-remove-backend",
    ) {
        return code;
    }
    let resolved = {
        let fs = ramfs::fs();
        fs.resolve(&path, 0)
    };
    match resolved {
        Some(node) if node != 0 => {
            if backing::is_disk_backed(node) {
                return -errno::EROFS;
            }
            let mut fs = ramfs::fs();
            if fs.nodes[node].kind == NodeKind::Dir && !fs.is_empty_dir(node) {
                return -errno::ENOTEMPTY;
            }
            fs.nodes[node].used = false;
            fs.nodes[node].content = Vec::new();
            0
        }
        Some(_) => -errno::EBUSY,
        None => -errno::ENOENT,
    }
}
'@ `
    "pub fn sys_mkdirat(dirfd: i32"

Replace-Exact `
    "src/compat/linux/mod.rs" `
    @'
        MKDIR => file::sys_mkdir(args[0]),
        MKDIRAT => file::sys_mkdir(args[1]),
        UNLINK => file::sys_unlink(args[0]),
        UNLINKAT => file::sys_unlink(args[1]),
'@ `
    @'
        MKDIR => file::sys_mkdir(args[0], args[1] as u32),
        MKDIRAT => file::sys_mkdirat(args[0] as i32, args[1], args[2] as u32),
        UNLINK => file::sys_unlink(args[0]),
        UNLINKAT => file::sys_unlinkat(args[0] as i32, args[1], args[2] as u32),
'@ `
    "file::sys_mkdirat(args[0] as i32"

# 8. Objects created through user syscalls are owned by the process effective
# credentials, not by the machine-wide login session.
Replace-Exact `
    "src/compat/linux/file.rs" `
    @'
            match fs.touch_at(parent, name) {
                Ok(node) => {
                    fs.nodes[node].mode = (mode & 0o777) as u16;
                    node
                }
                Err(raison) => return -errno_creation(raison),
            }
'@ `
    @'
            match fs.touch_at(parent, name) {
                Ok(node) => {
                    // SECURITY_OWNER_OPEN
                    let (owner_uid, owner_gid) =
                        crate::kernel::security::policy::filesystem_owner();
                    fs.nodes[node].mode = (mode & 0o777) as u16;
                    fs.nodes[node].uid = owner_uid;
                    fs.nodes[node].gid = owner_gid;
                    node
                }
                Err(raison) => return -errno_creation(raison),
            }
'@ `
    "SECURITY_OWNER_OPEN"

Replace-Exact `
    "src/compat/linux/file.rs" `
    @'
    let idx = match crate::fs::ramfs::fs().cree_anonyme(&nom) {
        Ok(idx) => idx,
        Err(_) => return -errno::ENFILE,
    };
    let process = task::current_process();
'@ `
    @'
    let idx = match crate::fs::ramfs::fs().cree_anonyme(&nom) {
        Ok(idx) => idx,
        Err(_) => return -errno::ENFILE,
    };
    {
        // SECURITY_OWNER_MEMFD
        let (owner_uid, owner_gid) =
            crate::kernel::security::policy::filesystem_owner();
        let mut fs = crate::fs::ramfs::fs();
        fs.nodes[idx].uid = owner_uid;
        fs.nodes[idx].gid = owner_gid;
    }
    let process = task::current_process();
'@ `
    "SECURITY_OWNER_MEMFD"

Write-Host ""
Write-Host "SECURITY_APPLY_V21_OK"
Write-Host "Ensuite:"
Write-Host "  python tools/security/verifie-security.py"
Write-Host "  git diff --check"
Write-Host "  .\tools\security\run-host-tests.ps1"
Write-Host "  cargo check"
Write-Host "  cargo bootimage"
Write-Host "  .\tools\security\run-security-ring3.ps1"
