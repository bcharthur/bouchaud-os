use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::sync::SpinLock;
use crate::kernel::task;

use super::capability::Capabilities;
use super::credentials::{CredentialError, Credentials};
use super::profile::{self, SecurityProfile};

#[derive(Clone)]
struct Entry {
    pid: u32,
    image: String,
    credentials: Credentials,
    capabilities: Capabilities,
    profile: SecurityProfile,
    no_new_privs: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    pub pid: u32,
    pub credentials: Credentials,
    pub capabilities: Capabilities,
    pub profile: SecurityProfile,
    pub no_new_privs: bool,
}

static CONTEXTS: SpinLock<Vec<Entry>> = SpinLock::new(Vec::new());
static CHECKS: AtomicU64 = AtomicU64::new(0);

fn make_entry(pid: u32, image: &str, uid: u32, gid: u32) -> Entry {
    let profile = profile::classify(image, uid);
    Entry {
        pid,
        image: image.to_string(),
        credentials: Credentials::new(uid, gid),
        capabilities: profile::initial_capabilities(image, uid),
        profile,
        no_new_privs: false,
    }
}

fn snapshot(entry: &Entry) -> Snapshot {
    Snapshot {
        pid: entry.pid,
        credentials: entry.credentials,
        capabilities: entry.capabilities,
        profile: entry.profile,
        no_new_privs: entry.no_new_privs,
    }
}

fn ensure_entry<'a>(
    contexts: &'a mut Vec<Entry>,
    pid: u32,
    image: &str,
    uid: u32,
    gid: u32,
) -> &'a mut Entry {
    let index = match contexts.iter().position(|entry| entry.pid == pid) {
        Some(index) => index,
        None => {
            contexts.push(make_entry(pid, image, uid, gid));
            contexts.len() - 1
        }
    };

    let entry = &mut contexts[index];

    // Legacy/session identity changes are synchronized into explicit credentials
    // but can never silently retain the stronger profile of the old identity.
    if entry.credentials.euid != uid || entry.credentials.egid != gid {
        entry.credentials = Credentials::new(uid, gid);
        let new_profile = profile::classify(image, uid);
        entry.capabilities =
            profile::transition_capabilities(entry.capabilities, new_profile);
        entry.profile = new_profile;
    }

    // execve keeps the PID but changes security domain. no_new_privs makes the
    // transition monotonic: exec may reduce authority, never increase it.
    if entry.image != image {
        let new_profile = profile::classify(image, entry.credentials.euid);
        entry.capabilities =
            profile::transition_capabilities(entry.capabilities, new_profile);
        entry.profile = new_profile;
        entry.image.clear();
        entry.image.push_str(image);
    }

    entry
}

fn maybe_maintenance() {
    let count = CHECKS.fetch_add(1, Ordering::Relaxed) + 1;
    if count & 0x3ff != 0 {
        return;
    }

    let live = task::processes();
    let mut contexts = CONTEXTS.lock();
    contexts.retain(|entry| live.iter().any(|process| process.pid == entry.pid));
}

pub fn current() -> Snapshot {
    let process = task::current_process();
    let metadata = process.metadata.lock();
    let pid = process.pid;
    let uid = metadata.uid;
    let gid = metadata.gid;
    let image = metadata.name.clone();

    let out = {
        let mut contexts = CONTEXTS.lock();
        snapshot(ensure_entry(
            &mut contexts,
            pid,
            image.as_str(),
            uid,
            gid,
        ))
    };
    drop(metadata);
    maybe_maintenance();
    out
}

pub fn launch(image: &str) -> Snapshot {
    if task::in_user_task() {
        return current();
    }

    let session = crate::users::session();
    let uid = session.uid() as u32;
    let gid = session.gid() as u32;
    let profile = profile::classify(image, uid);
    Snapshot {
        pid: 0,
        credentials: Credentials::new(uid, gid),
        capabilities: profile::initial_capabilities(image, uid),
        profile,
        no_new_privs: false,
    }
}

pub fn set_uid_current(target: u32) -> Result<Snapshot, CredentialError> {
    let process = task::current_process();
    let mut metadata = process.metadata.lock();
    let image = metadata.name.clone();
    let pid = process.pid;

    let out = {
        let mut contexts = CONTEXTS.lock();
        let entry = ensure_entry(
            &mut contexts,
            pid,
            image.as_str(),
            metadata.uid,
            metadata.gid,
        );
        entry.credentials.set_uid(target, entry.capabilities)?;

        metadata.uid = entry.credentials.euid;
        let new_profile = profile::classify(entry.image.as_str(), entry.credentials.euid);
        entry.capabilities =
            profile::transition_capabilities(entry.capabilities, new_profile);
        entry.profile = new_profile;
        snapshot(entry)
    };

    Ok(out)
}

pub fn set_gid_current(target: u32) -> Result<Snapshot, CredentialError> {
    let process = task::current_process();
    let mut metadata = process.metadata.lock();
    let image = metadata.name.clone();
    let pid = process.pid;

    let out = {
        let mut contexts = CONTEXTS.lock();
        let entry = ensure_entry(
            &mut contexts,
            pid,
            image.as_str(),
            metadata.uid,
            metadata.gid,
        );
        entry.credentials.set_gid(target, entry.capabilities)?;

        metadata.gid = entry.credentials.egid;
        snapshot(entry)
    };

    Ok(out)
}

pub fn set_no_new_privs_current() {
    let process = task::current_process();
    let metadata = process.metadata.lock();
    let mut contexts = CONTEXTS.lock();
    let entry = ensure_entry(
        &mut contexts,
        process.pid,
        metadata.name.as_str(),
        metadata.uid,
        metadata.gid,
    );
    entry.no_new_privs = true;
}

pub fn no_new_privs_current() -> bool {
    current().no_new_privs
}

pub fn apply_profile(pid: u32, wanted: SecurityProfile) -> bool {
    let Some(process) = task::process_by_pid(pid) else {
        return false;
    };
    let metadata = process.metadata.lock();
    let mut contexts = CONTEXTS.lock();
    let entry = ensure_entry(
        &mut contexts,
        pid,
        metadata.name.as_str(),
        metadata.uid,
        metadata.gid,
    );
    entry.capabilities =
        profile::transition_capabilities(entry.capabilities, wanted);
    entry.profile = wanted;
    true
}

pub fn inherit(parent_pid: u32, child_pid: u32) {
    if parent_pid == child_pid {
        return;
    }

    let Some(parent) = task::process_by_pid(parent_pid) else {
        return;
    };
    let Some(child) = task::process_by_pid(child_pid) else {
        return;
    };
    if child.parent != parent_pid {
        return;
    }

    let parent_metadata = parent.metadata.lock();
    let parent_image = parent_metadata.name.clone();
    let parent_uid = parent_metadata.uid;
    let parent_gid = parent_metadata.gid;
    drop(parent_metadata);

    let child_metadata = child.metadata.lock();
    let child_image = child_metadata.name.clone();
    drop(child_metadata);

    let mut contexts = CONTEXTS.lock();
    let inherited = {
        let parent_entry = ensure_entry(
            &mut contexts,
            parent_pid,
            parent_image.as_str(),
            parent_uid,
            parent_gid,
        );
        parent_entry.clone()
    };

    contexts.retain(|entry| entry.pid != child_pid);
    let mut child_entry = inherited;
    child_entry.pid = child_pid;
    child_entry.image = child_image;
    contexts.push(child_entry);
}

pub fn forget(pid: u32) {
    CONTEXTS.lock().retain(|entry| entry.pid != pid);
}

/// Effective owner stamped on filesystem objects created by a user syscall.
pub fn filesystem_owner() -> (u16, u16) {
    let security = current();
    (
        security.credentials.euid.min(u16::MAX as u32) as u16,
        security.credentials.egid.min(u16::MAX as u32) as u16,
    )
}
