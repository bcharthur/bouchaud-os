# C22 — isolated VMA ownership probe (2026-11-01)

Source: [PR #230](https://github.com/bcharthur/bouchaud-os/pull/230), head `6fef5d76df953a82914bf8306df86d7ac512824d`. This experimental branch is **not** a memory fix and must not replace the validated branch before CI.

## Observation

In KVM run [41271531625](https://github.com/bcharthur/bouchaud-os/actions/runs/41271531625), all three failing candidate trials (2, 3, 4) have a 132 KiB anonymous VMA `0x40000be1b000..0x40000be3c000`, absent in the three passing trials. This virtual mapping accounts for **at most 132 KiB of resident memory**, while candidate failures exceed the 1024 KiB RSS growth limit by more. Correlation does not prove causation or allocator ownership.

## Experiment

A bounded kernel trace in `src/compat/linux/mem.rs` logs PID 20 `mmap`, `munmap`, `madvise` and successful `mprotect` operations for **exactly 132 KiB** requests or ranges overlapping the suspect VMA. Fields: monotonic ns, PID, TID, operation, requested address, aligned length, protection, flags/advice, result. At most 64 records are printed; a `truncated=1` marker explicitly invalidates complete-coverage claims. No allocator-site backtrace is asserted. The exact 132 KiB size is a filter, not an assumption that the owner is mimalloc.

## Acceptance

1. Compile kernel and run existing strict 6x6 plus historical witness without modifying the 1024 KiB limit.
2. Check `[C22:MMAP]` entries in **raw serial logs**, not just parsed summaries.
3. Compare syscall records and VMA snapshots by PID and monotonic clock; classify mapping creator and whether `munmap`/`madvise` occurs.
4. If no trace appears, do **not** claim the mapping is unallocated: it may have been created by `brk`, an indirect `mremap`/ELF setup, an earlier event, or a non-PID20 process. The probe now covers successful `mprotect` operations that can split an existing VMA, but it still does not prove physical residency or a native allocation call site.
5. If 64 records are exhausted, treat the trace as incomplete. Separate allocation-site ownership from physical RSS residency; additional per-VMA page-table accounting is still needed.

No change to P13, kernel memory protections, C22 thresholds, witness mode or verdict semantics.

## Evidence recheck, 08:00 local

From the actual `ladybird-memoire` artifact of run 41271531625 (SHA `6fef5d76df953a82914bf8306df86d7ac512824d`), candidate outcomes are 3/6 green, historical witness has only 2/6 expected reds. The extra VMA is absent at 70 seconds and present at 75 seconds in candidate trials 2/3/4 only. Its 132 KiB virtual extent cannot explain 1,216–1,348 KiB of retained RSS. This branch is a diagnostic experiment, not a qualifying fix.

The diagnostic-only mimalloc statistics contain a *current* reserved figure of 1.0 GiB and committed falling from 523.9 MiB (cycle 16) to 176.8 MiB (cycle 80), while the kernel-measured WebContent RSS is ~70 MiB. The `pages` current field is negative (-105 to -363), so these figures cannot be treated as direct RSS or a reliable attribution without understanding mimalloc accounting and per-thread collection. We must not infer a native leak solely from these global counters.

The next decisive evidence is the successful syscall that maps or splits `0x40000be1b000..0x40000be3c000`, followed by per-VMA **resident** page counts and allocation-site sampling. If the 64-event limit is reached, report incomplete coverage. A new CI result from this experimental branch has not yet been confirmed.
