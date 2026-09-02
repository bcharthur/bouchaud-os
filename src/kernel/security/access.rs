use super::credentials::Credentials;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct AccessMask(pub u8);

impl AccessMask {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(0b100);
    pub const WRITE: Self = Self(0b010);
    pub const EXECUTE: Self = Self(0b001);

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Traditional Unix DAC check used by the security boundary.
///
/// `mode` contains the low permission bits (and may also contain sticky/set-id
/// bits). Root bypasses ordinary DAC, but executable FILES still need at least
/// one execute bit: an arbitrary data file does not become code just because
/// euid is zero. Directory traversal is allowed to the system profile.
pub fn mode_allows(
    creds: Credentials,
    owner: u32,
    group: u32,
    mode: u32,
    wanted: AccessMask,
    is_dir: bool,
) -> bool {
    if wanted == AccessMask::NONE {
        return true;
    }

    if creds.euid == 0 {
        if wanted.contains(AccessMask::EXECUTE) && !is_dir {
            return mode & 0o111 != 0;
        }
        return true;
    }

    let shift = if creds.euid == owner {
        6
    } else if creds.in_group(group) {
        3
    } else {
        0
    };
    let granted = ((mode >> shift) & 0o7) as u8;
    (granted & wanted.0) == wanted.0
}

/// Sticky directories (`/tmp`) allow removal/rename only by root, the
/// directory owner or the victim owner.
pub fn sticky_allows(
    creds: Credentials,
    parent_owner: u32,
    parent_mode: u32,
    victim_owner: u32,
) -> bool {
    if parent_mode & 0o1000 == 0 {
        return true;
    }
    creds.euid == 0 || creds.euid == parent_owner || creds.euid == victim_owner
}
