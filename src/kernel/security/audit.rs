use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::sync::SpinLock;

const MAX_EVENTS: usize = 256;

#[derive(Clone, Copy, Debug)]
pub struct AuditEvent {
    pub sequence: u64,
    pub time_ns: u64,
    pub pid: u32,
    pub uid: u32,
    pub operation: &'static str,
    pub detail: u64,
}

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static DENIALS: AtomicU64 = AtomicU64::new(0);
static EVENTS: SpinLock<Vec<AuditEvent>> = SpinLock::new(Vec::new());

fn record(pid: u32, uid: u32, operation: &'static str, detail: u64) -> u64 {
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
    DENIALS.fetch_add(1, Ordering::Relaxed);

    let mut events = EVENTS.lock();
    if events.len() == MAX_EVENTS {
        events.remove(0);
    }
    events.push(AuditEvent {
        sequence,
        time_ns: crate::kernel::timer::monotonic_ns(),
        pid,
        uid,
        operation,
        detail,
    });
    sequence
}

pub fn deny(pid: u32, uid: u32, operation: &'static str, detail: u64) {
    let sequence = record(pid, uid, operation, detail);
    crate::kernel::dmesg::log_fmt(format_args!(
        "[SECURITY-DENY] seq={} pid={} uid={} op={} detail={:#x}",
        sequence, pid, uid, operation, detail,
    ));
}

/// Path-aware denial.  The bounded ring keeps the compact numeric event while
/// dmesg receives the canonical target and a stable reason string, which makes
/// browser sandbox failures diagnosable without weakening policy.
pub fn deny_path(
    pid: u32,
    uid: u32,
    operation: &'static str,
    detail: u64,
    canonical_path: &str,
    reason: &'static str,
) {
    let sequence = record(pid, uid, operation, detail);
    crate::kernel::dmesg::log_fmt(format_args!(
        "[SECURITY-DENY] seq={} pid={} uid={} op={} detail={:#x} path={} reason={}",
        sequence, pid, uid, operation, detail, canonical_path, reason,
    ));
}

pub fn denial_count() -> u64 {
    DENIALS.load(Ordering::Relaxed)
}

pub fn snapshot() -> Vec<AuditEvent> {
    EVENTS.lock().clone()
}
