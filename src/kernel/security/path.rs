use alloc::string::String;
use alloc::vec::Vec;

/// Lexically canonicalise an absolute Unix path.
///
/// Bouchaud's RAMFS currently has no symbolic links, so resolving `.` and `..`
/// is sufficient to make the path seen by the policy identical to the target
/// reached by the filesystem resolver.  If symlinks are added later, this
/// contract must move to inode-based resolution rather than staying lexical.
pub fn normalize_absolute(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                let _ = parts.pop();
            }
            _ => parts.push(part),
        }
    }

    let mut out = String::from("/");
    for (index, part) in parts.iter().enumerate() {
        if index != 0 {
            out.push('/');
        }
        out.push_str(part);
    }
    out
}

/// Canonicalise `path` relative to an already-canonical absolute base.
/// Absolute `path` values deliberately ignore `base`, matching *at(2).
pub fn canonical_from_base(base: &str, path: &str) -> String {
    if path.starts_with('/') {
        return normalize_absolute(path);
    }

    let mut joined = if base.is_empty() {
        String::from("/")
    } else {
        normalize_absolute(base)
    };
    if !joined.ends_with('/') {
        joined.push('/');
    }
    joined.push_str(path);
    normalize_absolute(joined.as_str())
}
