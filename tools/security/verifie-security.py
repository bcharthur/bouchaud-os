#!/usr/bin/env python3
from pathlib import Path
import sys

errors = []

required = [
    "src/kernel/security/mod.rs",
    "src/kernel/security/access.rs",
    "src/kernel/security/audit.rs",
    "src/kernel/security/capability.rs",
    "src/kernel/security/credentials.rs",
    "src/kernel/security/execution.rs",
    "src/kernel/security/filesystem.rs",
    "src/kernel/security/memory.rs",
    "src/kernel/security/network.rs",
    "src/kernel/security/path.rs",
    "src/kernel/security/policy.rs",
    "src/kernel/security/profile.rs",
    "src/kernel/security/sandbox.rs",
    "src/kernel/security/syscall.rs",
    "tools/security/security-ring3-probe.c",
    "tools/security/prebuilt/security-ring3-probe",
    "tools/security/run-security-ring3.ps1",
    ".github/workflows/security-runtime.yml",
]
for name in required:
    if not Path(name).is_file():
        errors.append(f"{name}: absent")

def text(path):
    p = Path(path)
    return p.read_text(encoding="utf-8") if p.is_file() else ""

kernel = text("src/kernel/mod.rs")
usermode = text("src/arch/x86_64/usermode.rs")
exec_rs = text("src/kernel/process/exec.rs")
proc_rs = text("src/compat/linux/proc.rs")
processus = text("src/kernel/process/thread/processus.rs")
ramfs = text("src/fs/ramfs.rs")
file_rs = text("src/compat/linux/file.rs")
linux_mod = text("src/compat/linux/mod.rs")
security = "\n".join(text(p) for p in required if p.startswith("src/kernel/security/"))

contracts = [
    ("pub mod security;", "src/kernel/mod.rs", kernel),
    ("BOUCHAUD_SECURITY_V1", "src/arch/x86_64/usermode.rs", usermode),
    ("security::syscall::gate", "src/arch/x86_64/usermode.rs", usermode),
    ("security::syscall::after_syscall", "src/arch/x86_64/usermode.rs", usermode),
    ("security::execution::authorize_node", "src/kernel/process/exec.rs", exec_rs),
    ("canonical_from_node", "kernel-side canonical exec path", security),
    ("canonical_from_base", "pure canonical path engine", security),
    ("deny_path", "path-aware audit", security),
    ("security::filesystem::canonical_at", "src/compat/linux/proc.rs", proc_rs),
    ("security::policy::forget(self.pid)", "process lifecycle cleanup", processus),
    ("self.nodes[tmp].mode = 0o1777;", "sticky /tmp", ramfs),
    ("SECURITY_OWNER_OPEN", "created-file effective ownership", file_rs),
    ("SECURITY_DIRFD_BACKEND_V21", "dirfd-aware filesystem backend", file_rs),
    ("security_recheck_open(path.as_str(), flags)", "resolved-target security recheck", file_rs),
    ("pub fn sys_mkdirat(dirfd: i32", "mkdirat preserves dirfd", file_rs),
    ("pub fn sys_unlinkat(dirfd: i32", "unlinkat preserves dirfd", file_rs),
    ("SECURITY_OWNER_MKDIR", "created-dir effective ownership", file_rs),
    ("SECURITY_OWNER_MEMFD", "memfd effective ownership", file_rs),
    ("file::sys_mkdirat(args[0] as i32", "mkdirat dispatcher preserves dirfd", linux_mod),
    ("file::sys_unlinkat(args[0] as i32", "unlinkat dispatcher preserves dirfd", linux_mod),
    ("Capabilities::SET_IDENTITY", "capability enforcement", security),
    ("transition_capabilities", "monotonic authority", security),
    ("initial_capabilities", "identity-bounded initial authority", security),
    ("WriteExecute", "strict W^X", security),
    ("AnonymousExecute", "JIT boundary", security),
    ("mmap-file-access", "file mapping DAC", security),
    ("StickyDirectory", "sticky directory semantics", security),
    ("TKILL", "thread signal authorization", security),
    ("TGKILL", "thread-group signal authorization", security),
    ("[SECURITY-DENY]", "security audit", security),
    ("BrowserContent", "browser sandbox", security),
    ("PR_SET_NO_NEW_PRIVS", "security no_new_privs", security),
]
for needle, where, body in contracts:
    if needle not in body:
        errors.append(f"{where}: contrat absent: {needle}")


# All path-bearing *at policy calls must carry their real dirfd.
syscall_rs = text("src/kernel/security/syscall.rs")
for needle in [
    "args[0] as i32, args[1], filesystem::Mutation::Create",
    "args[0] as i32, args[1], filesystem::Mutation::Remove",
    "args[0] as i32, args[1], filesystem::Mutation::RenameSource",
    "args[2] as i32, args[3], filesystem::Mutation::RenameTarget",
]:
    if needle not in syscall_rs:
        errors.append(f"syscall.rs: dirfd-aware path gate absent: {needle}")

filesystem_rs = text("src/kernel/security/filesystem.rs")
if "FdKind::Dir(node) => Some(*node)" not in filesystem_rs:
    errors.append("filesystem.rs: dirfd must resolve only FdKind::Dir")
if "FdKind::Dir(node) | FdKind::File(node)" in filesystem_rs.split("fn base_node_for_dirfd", 1)[-1].split("pub fn canonical_from_node", 1)[0]:
    errors.append("filesystem.rs: regular file still accepted as dirfd")

# Security core must not regress to the historical global kernel lock or unsafe
# mutable globals. Its state is behind explicit subsystem locks.
for path in Path("src/kernel/security").glob("*.rs"):
    body = path.read_text(encoding="utf-8")
    if "smp_lock::enter" in body or "Domaine::Syscall" in body:
        errors.append(f"{path}: dependance interdite au BKL global")
    if "static mut " in body:
        errors.append(f"{path}: static mut interdit dans le coeur securite")

# Path classification must fail closed: untrusted location is tested before any
# browser-name rule, otherwise /tmp/BrowserHost would be an elevation primitive.
profile = text("src/kernel/security/profile.rs")
if profile.find("if untrusted_path(image)") > profile.find("if browser_content_image(image)"):
    errors.append("profile.rs: untrusted_path doit preceder les noms navigateur")
if "ambient.intersection" not in profile:
    errors.append("profile.rs: les capacites initiales ne sont pas bornees par l'identite")

if errors:
    for error in errors:
        print("ECHEC:", error, file=sys.stderr)
    raise SystemExit(1)

print("SECURITY_ARCH_V21_OK")
