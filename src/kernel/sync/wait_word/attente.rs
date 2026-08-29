// Consumer path. Ticket is captured BEFORE the value check.

pub fn wait_word_wait(uaddr: u64, expected: u32, timeout_ms: u64) -> WaitWordWake {
    let Some(key) = wait_word_key(uaddr) else {
        WW_FAULTS.fetch_add(1, Ordering::Relaxed);
        return WaitWordWake::Fault;
    };
    let entry = wait_word_entry(key);
    entry.waiters.fetch_add(1, Ordering::AcqRel);
    WW_WAITS.fetch_add(1, Ordering::Relaxed);

    let ticket = entry.wait.ticket();
    let current = match wait_word_read(uaddr) {
        Some(value) => value,
        None => {
            entry.waiters.fetch_sub(1, Ordering::AcqRel);
            WW_FAULTS.fetch_add(1, Ordering::Relaxed);
            prune_wait_word(&entry);
            return WaitWordWake::Fault;
        }
    };

    if current != expected {
        entry.waiters.fetch_sub(1, Ordering::AcqRel);
        WW_VALUE_CHANGED.fetch_add(1, Ordering::Relaxed);
        prune_wait_word(&entry);
        return WaitWordWake::ValueChanged;
    }

    let wake = if timeout_ms == 0 {
        entry.wait.wait(ticket)
    } else {
        let deadline = crate::kernel::timer::monotonic_ns()
            .saturating_add(timeout_ms.saturating_mul(1_000_000));
        entry.wait.wait_until(ticket, deadline)
    };

    entry.waiters.fetch_sub(1, Ordering::AcqRel);
    let result = match wake {
        WaitSourceWake::Deadline => {
            WW_DEADLINES.fetch_add(1, Ordering::Relaxed);
            WaitWordWake::Deadline
        }
        WaitSourceWake::Signaled | WaitSourceWake::AlreadyChanged => {
            WW_SIGNALED.fetch_add(1, Ordering::Relaxed);
            WaitWordWake::Signaled
        }
    };
    prune_wait_word(&entry);
    result
}
