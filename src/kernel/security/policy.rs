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
        // BOUCHAUD_C6_NO_NEW_PRIVS_IMPLICITE_V1
        //
        // Un role sandboxe ne doit pas avoir a DEMANDER `no_new_privs` : il
        // faudrait qu'il le fasse lui-meme, or c'est precisement le processus
        // dont on suppose qu'il peut etre compromis. Le drapeau est donc pose
        // par le profil, avant que le programme ne tourne.
        no_new_privs: profile::sandboxe(profile),
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
        // Un exec vers un role sandboxe pose le drapeau ; il ne le retire
        // JAMAIS. `no_new_privs` est monotone, sinon un exec suffirait a s'en
        // debarrasser -- ce qui serait exactement le contraire de son objet.
        let (new_profile, capacites, nnp) = profile::transition_exec(
            entry.capabilities,
            entry.no_new_privs,
            image,
            entry.credentials.euid,
        );
        entry.capabilities = capacites;
        entry.profile = new_profile;
        entry.no_new_privs = nnp;
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
    let instantane = current();
    // BOUCHAUD_NNP_DIAGNOSTIC_V1 : endurance 37627107473, deux WebWorker sur
    // vingt-neuf lisent `no_new_privs=0` et refusent de tourner
    // (`[LB:SANDBOX] ECHEC`). Le profil vient de l'image ; dire laquelle le
    // noyau a vue, et sous quel profil, au moment ou il repond 0.
    if !instantane.no_new_privs {
        let process = task::current_process();
        let image = process.metadata.lock().name.clone();
        crate::kernel::dmesg::log_fmt(format_args!(
            "NNP_ABSENT pid={} image={} profil={:?} euid={}",
            instantane.pid, image, instantane.profile, instantane.credentials.euid
        ));
    }
    instantane.no_new_privs
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
    let child_uid = child_metadata.uid;
    let child_gid = child_metadata.gid;
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

    // BOUCHAUD_HERITAGE_APRES_EXEC_V1
    //
    // Cet heritage s'execute dans le PERE, au retour de clone/fork. Avec
    // posix_spawn (CLONE_VM|CLONE_VFORK, que le noyau ne suspend pas), le
    // fils peut avoir DEJA execute sa nouvelle image. L'ancien code collait
    // l'entree du pere sous l'image COURANTE du fils : image=WebWorker,
    // profil BrowserBroker, no_new_privs=0 -- et comme l'image ne
    // « changeait » plus, aucun exec ne la reclassait. Un processus de rendu
    // tournait avec les droits du courtier (endurance 37654172489 :
    // `NNP_ABSENT pid=34 image=/usr/libexec/ladybird/WebWorker
    // profil=BrowserBroker`, 3 WebWorker sur 29).
    //
    // Le fils herite de l'etat du pere AU MOMENT DU FORK (image du pere),
    // puis la transition d'exec s'applique vers son image reelle, par le
    // meme chemin qu'un exec : reclassement, droits intersectes,
    // no_new_privs monotone.
    // Ce que le fils a deja pose lui-meme (prctl) ne se perd pas non plus :
    // no_new_privs ne redescend jamais.
    let deja_confine = contexts.iter().any(|entry| entry.pid == child_pid && entry.no_new_privs);
    contexts.retain(|entry| entry.pid != child_pid);
    let mut child_entry = inherited;
    child_entry.pid = child_pid;
    contexts.push(child_entry);
    let entree = ensure_entry(&mut contexts, child_pid, child_image.as_str(), child_uid, child_gid);
    entree.no_new_privs |= deja_confine;
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
