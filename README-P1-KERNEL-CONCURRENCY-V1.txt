P1 SCRIPT HOTFIX V1.1
---------------------
The first P1 package had a PowerShell-only variable-name collision:
`$LinuxBkl` (path) and `$linuxBkl` (file contents) are the same variable
because PowerShell variable names are case-insensitive. The apply script
therefore tried to use the Rust source text itself as a backup path.

The failure happened during backup creation, before any source file write.
The missing P1 marker reported by VERIFY confirms that the P1 source patch was
not partially applied.

This v1.1 package renames them to `$LinuxBklPath` and `$linuxBklSource`.

BOUCHAUD OS - P1 KERNEL CONCURRENCY V1
=======================================

WHY THIS PATCH
--------------
P0 v1.4 proved that the SMP wake path is now correct: idle APs stay quiet and
Ladybird can start and load example.com.

The next bottleneck is no longer "can a CPU wake?". It is "can four CPUs make
kernel progress at the same time?".

The validated run shows:
- multi-second cumulative BKL waiting inside five-second windows;
- large WaitQueue BKL wait totals;
- hundreds of migrations in a five-second sample;
- poll (syscall 7), futex (202) and execve (59) visible in long kernel phases.

P1 V1 CHANGES
-------------
1. BKL parking has its own wake protocol.
   The old BKL adaptive HLT still depended on periodic scheduler interrupts.
   P0 removed those periodic interrupts. P1 publishes parked BKL waiters with
   IF=0, rechecks OWNER, and wakes them explicitly on every final BKL release.

2. BKL active-spin threshold: 64 -> 512.
   Very short critical sections no longer generate an immediate HLT/IPI
   round-trip under TCG.

3. WaitQueue zero-waiter fast path.
   notify_readiness() still advances its generation, but if no task is actually
   parked on that queue it no longer enters the BKL and scans TASKS.

4. poll/ppoll no longer keep the BKL across the whole readiness scan/sleep.
   FileTable, Mm, pipe/socketpair/eventfd/timerfd objects already have their own
   synchronization domains. The legacy PS/2 and TcpConn pump probes still take
   a short explicit BKL. WaitQueue takes the BKL only at the TASKS park/wake
   boundary.

5. Scheduler migration hysteresis.
   Minimum cache residency moves from 20 ms to 250 ms.
   Failed/successful work stealing gets a 10 ms retry cooldown.
   Local work is still preferred and normal load balancing is preserved.

6. Better BKL observability.
   BKL-STATS gains:
     parked_waiters
     parks
     wake_ipis
   max_hold_site falls back to acquisition provenance when the live site was
   already cleared.

7. The external BKL verifier is repaired for the new src/compat/linux layout
   and POLL/PPOLL receive explicit named audits.

WHAT THIS PATCH DELIBERATELY DOES NOT FAKE
-----------------------------------------
This is not "the BKL is gone".

- FUTEX still needs the TASKS registry for park/wake.
- EXECVE preparation still reads the RAMFS/ELF path whose read-side
  synchronization has not yet been proven independent of the BKL.
- FD readiness is still one global generation queue; when many tasks are
  actually waiting, wake_all can still produce a thundering herd.
- TASKS is still a global mutable Vec and is the structural reason several
  paths cannot yet be made lock-free.

Those need a second structural phase. Marking them Sans-BKL now would be fast
until the first SMP corruption. P1 V1 intentionally takes the largest safe
steps that can be measured in isolation.

APPLICATION
-----------
This patch expects the already validated P0 v1.2 + v1.3 + v1.4 local tree.

From repo root:

  .\APPLY-P1-KERNEL-CONCURRENCY-V1.ps1 -Preview
  .\APPLY-P1-KERNEL-CONCURRENCY-V1.ps1
  .\VERIFY-P1-KERNEL-CONCURRENCY-V1.ps1 -Build
  .\run.ps1

A byte-for-byte backup is created in:

  .bouchaud-history\backups\.bouchaud-p1-kernel-concurrency-v1-<timestamp>\

A/B TEST
--------
Use the same workload as the validated P0 test:

1. Boot.
2. Idle 10 seconds.
3. Open Ladybird.
4. Wait for https://example.com/ to fully load.
5. Leave it untouched for another 20-30 seconds.
6. Save the complete log.

Compare:
- BKL-STATS wait_ns / hold_ns
- SMP-SAMPLE bkl_wait_delta_ns
- MM-NG6 waitq_bkl_enters / waitq_bkl_wait_ns
- SMP-SAMPLE mig_delta
- SMP-LOAD per-CPU migration totals
- BKL-STATS parks / wake_ipis / parked_waiters
- max_hold_ns / max_hold_site
- PERF_EXEC_START -> BROWSER_HOST_INITIALIZED
- PERF_EXEC_START -> M11_DOCUMENT_LOADED

SUCCESS CRITERIA
----------------
Correctness:
- idle AP scheduler IPIs still stay almost flat;
- no rq=1 lost-wakeup regression;
- BrowserHost, WebContent and services start;
- example.com renders;
- keyboard/mouse/text selection still work.

Performance:
- migration deltas should fall materially;
- poll no longer appears as one long outer BKL ownership interval;
- waitq BKL entries should grow more slowly when no one is parked;
- BKL cumulative wait should be lower for the same browser workload.

ROLLBACK
--------
  .\ROLLBACK-P1-KERNEL-CONCURRENCY-V1.ps1

DO NOT COMMIT YET
-----------------
Run the A/B test first. If it is stable and metrics improve, then commit P0+P1
as a measured SMP milestone.
