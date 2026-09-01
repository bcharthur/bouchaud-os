use super::policy;
use super::profile::SecurityProfile;

pub fn browser_broker(pid: u32) -> bool {
    policy::apply_profile(pid, SecurityProfile::BrowserBroker)
}

pub fn browser_content(pid: u32) -> bool {
    policy::apply_profile(pid, SecurityProfile::BrowserContent)
}

pub fn untrusted(pid: u32) -> bool {
    policy::apply_profile(pid, SecurityProfile::Untrusted)
}

pub fn current_profile() -> SecurityProfile {
    policy::current().profile
}
