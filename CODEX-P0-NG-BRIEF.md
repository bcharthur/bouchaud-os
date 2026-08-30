# Bouchaud OS — P0 NG / Codex brief

Base: `main@023f74f4f02025466a9ea119ad14bd6199e7e65b`
Branch: `perf/p0-ng-codex`

## Objective

Turn the current performance/concurrency foundation into three real P0 architectural upgrades, without hiding failures behind relaxed assertions.

### P0.1 — BKL -> genuinely concurrent kernel

The final goal is not a faster BKL. The normal Ladybird path must progressively stop depending on the global BKL.

Audit and migrate, in small measurable steps:

1. process / task registry
2. FD tables and per-process state
3. futex / wait paths
4. VM / VMA / page faults
5. VFS / inode / file state
6. sockets / network queues
7. drivers

Rules:
- object/subsystem locks instead of global serialization;
- explicit lock ranking;
- runtime lockdep in debug builds;
- never sleep while holding a spinlock;
- do not remove ownership/debug assertions just to make a test pass;
- do not treat file/module fragmentation as BKL removal;
- every migration must expose BKL acquisitions, wait time and max hold time.

Known runtime failure from the experimental NG1.1 work:
- SMP4 Ladybird eventually panicked with `smp_lock: release par un CPU non proprietaire`;
- flight recorder observed a same-CPU `REENTER` while local BKL depth was 0;
- CPU ownership alone is therefore not a sufficient reentrancy identity across scheduler suspend/resume paths.

Treat this as an architectural invariant bug, not as a panic to suppress.

### P0.2 — Scheduler NG + safe kernel preemption

The repository already contains SMP/runqueue/affinity/work-stealing foundations, but the historical task scheduler documents the kernel as non-preemptible and runtime integration is incomplete.

Target, progressively:
- true per-CPU runnable work;
- deferred kernel preemption at explicit safe points;
- `need_resched` per CPU;
- no switch while IRQ depth != 0;
- no switch while `preempt_count != 0`;
- no switch while spin/ranked locks are held;
- no switch with lockdep depth != 0;
- wake-to-run latency instrumentation;
- priority-aware queues;
- explicit interactive policy for input/compositor/browser UI;
- migration throttling to preserve cache locality;
- later: deadline/tickless timer design.

Do not claim scheduler NG complete until runtime metrics prove requests, safe-points and actual switches under Ladybird.

### P0.3 — Memory NG

Target architecture:
- physical buddy allocator or equivalent bounded allocator;
- per-CPU frame magazines/caches;
- slab/size classes for kernel objects;
- fast per-CPU alloc/free paths with global fallback;
- page-cache working-set policy;
- normal / low / critical pressure watermarks;
- clean page reclaim;
- deterministic OOM path;
- batched TLB shootdowns.

A per-CPU cache in front of `LockedHeap` is a useful transitional optimization, not the final memory architecture.

## Existing repository signals

Read before editing:
- `AUDIT-FRAGMENTATION-2026-08-29.md`
- `BOUCHAUD-PERF-OBSERVATORY-V1.md`
- `src/kernel/process/thread.rs`
- `src/kernel/sync/bkl/`
- `src/kernel/debug/perf/`
- `.github/workflows/ci.yml`

The current fragmentation audit already identifies futex, readiness/poll, network and demand paging as important next boundaries.

## Required development discipline

For each semantic change:
1. keep the patch narrow;
2. run `cargo check`;
3. run repository lock/BKL validators;
4. build `cargo bootimage` when the environment supports it;
5. preserve/assert invariants;
6. add runtime counters before optimizing the next subsystem.

No giant rewrite of BKL + scheduler + VM in one unreviewable patch.

## Runtime acceptance gates

A local QEMU/Ladybird SMP4 stress run is the final gate. Target initially:

- no kernel panic;
- no double fault;
- lockdep violations = 0;
- BKL ownership violations = 0;
- BKL max hold < 100 ms as an intermediate target, then < 10 ms;
- scheduler NG requests > 0;
- scheduler NG safe-points > 0;
- scheduler NG switches > 0;
- interactive wake-to-run latency measured;
- per-CPU heap/frame cache hit ratios measured;
- no unexplained OOM or memory corruption.

## CI issue to fix separately

Workflow run `99279296274` spent roughly three hours in `Gate0 runtime` before failing. Do not optimize Cargo first: Gate0 must be fail-fast, terminate QEMU reliably, stop immediately on panic, stop successfully once mandatory markers are observed, have a short PR timeout, and upload the serial log on failure.

Keep the long stress workload outside the normal PR Gate0.

## Merge policy

This branch is experimental. Do not merge to `main` merely because it compiles. The P0 work is accepted only after the runtime gates above pass and the normal Ladybird path demonstrates decreasing BKL dependence.
