extern crate alloc;

mod capability {
    include!("../../src/kernel/security/capability.rs");
}
mod credentials {
    include!("../../src/kernel/security/credentials.rs");
}
mod profile {
    include!("../../src/kernel/security/profile.rs");
}
mod access {
    include!("../../src/kernel/security/access.rs");
}
mod path {
    include!("../../src/kernel/security/path.rs");
}

use access::{mode_allows, sticky_allows, AccessMask};
use capability::Capabilities;
use credentials::{CredentialError, Credentials};
use profile::{
    capabilities, classify, initial_capabilities, transition_capabilities,
    SecurityProfile,
};

const PROT_WRITE: u32 = 2;
const PROT_EXEC: u32 = 4;

fn protection_allowed(caps: Capabilities, prot: u32, anonymous: bool) -> bool {
    if prot & PROT_WRITE != 0 && prot & PROT_EXEC != 0 {
        return false;
    }
    if prot & PROT_EXEC != 0 && !caps.contains(Capabilities::EXEC) {
        return false;
    }
    if prot & PROT_EXEC != 0 && anonymous && !caps.contains(Capabilities::JIT) {
        return false;
    }
    true
}

#[test]
fn strict_wx_even_for_system() {
    assert!(!protection_allowed(
        capabilities(SecurityProfile::System),
        PROT_WRITE | PROT_EXEC,
        true
    ));
}

#[test]
fn anonymous_exec_is_a_jit_capability() {
    let user = capabilities(SecurityProfile::User);
    assert!(!protection_allowed(user, PROT_EXEC, true));
    let system = capabilities(SecurityProfile::System);
    assert!(protection_allowed(system, PROT_EXEC, true));
}

#[test]
fn browser_content_is_not_device_or_admin() {
    let caps = capabilities(SecurityProfile::BrowserContent);
    assert!(caps.contains(Capabilities::IPC_TRANSFER));
    assert!(caps.contains(Capabilities::JIT));
    assert!(!caps.contains(Capabilities::DEVICE_IO));
    assert!(!caps.contains(Capabilities::SYSTEM_ADMIN));
    assert!(!caps.contains(Capabilities::SET_IDENTITY));
}

#[test]
fn untrusted_location_wins_over_browser_name() {
    assert_eq!(
        classify("/tmp/BrowserHost", 0),
        SecurityProfile::Untrusted
    );
}

#[test]
fn non_root_cannot_gain_broker_device_capability_by_exec_name() {
    let caps = initial_capabilities("/usr/bin/bo-navigateur", 1000);
    assert!(caps.contains(Capabilities::EXEC));
    assert!(!caps.contains(Capabilities::DEVICE_IO));
    assert!(!caps.contains(Capabilities::SYSTEM_ADMIN));
}

#[test]
fn root_launch_of_trusted_broker_is_deliberately_reduced() {
    let caps = initial_capabilities("/usr/bin/bo-navigateur", 0);
    assert!(caps.contains(Capabilities::DEVICE_IO));
    assert!(caps.contains(Capabilities::JIT));
    assert!(caps.contains(Capabilities::IPC_TRANSFER));
    assert!(!caps.contains(Capabilities::SYSTEM_ADMIN));
}

#[test]
fn profile_transition_never_manufactures_authority() {
    let user = capabilities(SecurityProfile::User);
    let attempted_system = transition_capabilities(user, SecurityProfile::System);
    assert_eq!(attempted_system, user);
    assert!(!attempted_system.contains(Capabilities::SYSTEM_ADMIN));
}

#[test]
fn privileged_setuid_is_irreversible_after_cap_drop() {
    let mut creds = Credentials::new(0, 0);
    creds.set_uid(1000, Capabilities::ALL).unwrap();
    assert_eq!(creds.euid, 1000);
    assert_eq!(
        creds.set_uid(0, capabilities(SecurityProfile::User)),
        Err(CredentialError::PermissionDenied)
    );
}

#[test]
fn unix_mode_owner_group_other_is_enforced() {
    let owner = Credentials::new(1000, 1000);
    let other = Credentials::new(2000, 2000);
    assert!(mode_allows(owner, 1000, 1000, 0o640, AccessMask::READ, false));
    assert!(mode_allows(owner, 1000, 1000, 0o640, AccessMask::WRITE, false));
    assert!(!mode_allows(other, 1000, 1000, 0o640, AccessMask::READ, false));
}

#[test]
fn root_still_needs_an_execute_bit_for_files() {
    let root = Credentials::new(0, 0);
    assert!(!mode_allows(root, 1000, 1000, 0o644, AccessMask::EXECUTE, false));
    assert!(mode_allows(root, 1000, 1000, 0o744, AccessMask::EXECUTE, false));
}

#[test]
fn sticky_directory_protects_other_users_files() {
    let alice = Credentials::new(1000, 1000);
    let root = Credentials::new(0, 0);
    assert!(!sticky_allows(alice, 0, 0o1777, 1001));
    assert!(sticky_allows(alice, 0, 0o1777, 1000));
    assert!(sticky_allows(root, 0, 0o1777, 1001));
}

#[test]
fn untrusted_profile_has_minimal_authority() {
    let caps = capabilities(SecurityProfile::Untrusted);
    assert!(caps.contains(Capabilities::EXEC));
    assert!(!caps.contains(Capabilities::IPC_TRANSFER));
    assert!(!caps.contains(Capabilities::DEVICE_IO));
    assert!(!caps.contains(Capabilities::DEBUG));
}


#[test]
fn canonical_path_collapses_dotdot_before_policy() {
    assert_eq!(path::normalize_absolute("/tmp/a/../ok"), "/tmp/ok");
    assert_eq!(path::normalize_absolute("/tmp/../../etc/x"), "/etc/x");
    assert_eq!(path::normalize_absolute("//dev/./fb0"), "/dev/fb0");
}

#[test]
fn canonical_relative_path_uses_exact_dirfd_base() {
    assert_eq!(path::canonical_from_base("/tmp", "x"), "/tmp/x");
    assert_eq!(
        path::canonical_from_base("/persist/private", "../../tmp/x"),
        "/tmp/x"
    );
    assert_eq!(
        path::canonical_from_base("/tmp", "/dev/fb0"),
        "/dev/fb0"
    );
}
