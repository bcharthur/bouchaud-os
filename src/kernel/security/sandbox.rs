use super::policy;
use super::profile::SecurityProfile;

pub fn browser_broker(pid: u32) -> bool {
    policy::apply_profile(pid, SecurityProfile::BrowserBroker)
}

pub fn browser_content(pid: u32) -> bool {
    policy::apply_profile(pid, SecurityProfile::BrowserContent)
}

/// Le role qui possede le reseau du navigateur.
pub fn browser_network(pid: u32) -> bool {
    policy::apply_profile(pid, SecurityProfile::BrowserNetwork)
}

pub fn untrusted(pid: u32) -> bool {
    policy::apply_profile(pid, SecurityProfile::Untrusted)
}

pub fn current_profile() -> SecurityProfile {
    policy::current().profile
}
