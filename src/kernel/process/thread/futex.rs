// --- Wait-word compatibility bridge -----------------------------------------
//
// The task subsystem no longer owns futex state. Linux/POSIX calls still use
// the historical `task::futex_*` API, but the real mechanism is the Bouchaud
// native wait-word core in `kernel::sync::wait_word`.

pub fn futex_wait(uaddr: u64, expected: u32, timeout_ms: u64) -> bool {
    // The Linux syscall may still enter through the conservative outer-BKL
    // policy. Explicitly suspend it for the native wait. V13 can therefore be
    // benchmarked without weakening the default syscall safety table globally.
    let depth = smp_lock::suspend_for_schedule();
    let result = crate::kernel::sync::wait_word_wait(uaddr, expected, timeout_ms);
    smp_lock::resume_after_schedule(depth);
    matches!(
        result,
        crate::kernel::sync::WaitWordWake::Signaled
            | crate::kernel::sync::WaitWordWake::ValueChanged
    )
}

pub fn futex_wake(uaddr: u64, count: u32) -> u32 {
    let depth = smp_lock::suspend_for_schedule();
    let result = crate::kernel::sync::wait_word_wake(uaddr, count);
    smp_lock::resume_after_schedule(depth);
    result
}
