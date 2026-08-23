# SMP-NG foundation — NG1

This milestone changes the CPU model without yet changing scheduling semantics.

## What it fixes now

The previous SMP4 bring-up used the QEMU APIC ID as the runtime CPU array index.
That works for `0,1,2,3`, but it is not a valid long-term SMP invariant.

NG1 introduces a dense Bouchaud `CpuId` namespace and a CPU registry:

- `CpuId`: logical CPU in `0..MAX_CPUS`
- `CpuDescriptor`: APIC ID + package/core/thread topology
- `CpuLocal`: per-logical-CPU state prepared for scheduler/IRQ/TLB ownership
- `CpuMask`: affinity/online-set building block
- CPUID topology parsing using leaf `0x1f`, then `0x0b`, then a legacy fallback
- APIC-ID -> logical-CpuId translation
- SMP topology diagnostic lines

Once an AP reaches Rust, GDT/TSS/GS and runtime arrays now use its dense logical
CPU ID rather than its hardware APIC ID.

## Intentional remaining bootstrap limitation

The 16/64-bit real-mode SIPI trampoline is intentionally unchanged in NG1.
Before Rust starts, it still selects the bootstrap stack from the legacy 8-bit
APIC ID and therefore still expects a bootstrap APIC ID below `MAX_CPUS`.

This is now isolated as a bootstrap-only limitation. A later topology/bootstrap
milestone can replace the raw trampoline mailbox with an APIC-ID lookup table
without mixing that risky change with the runtime CPU refactor.

## New kernel synchronization layer

`kernel::sync` now provides:

- `SpinLock<T>`
- `SpinLockIrq<T>`

The BKL remains the migration safety net. NG1 does not replace BKL call sites
yet. These primitives are the basis for moving each subsystem to narrow locks.

Rules:

- never sleep while holding `SpinLock`
- never call `schedule()` while holding `SpinLock`
- use `SpinLockIrq` only for state shared with an IRQ handler
- release `SpinLockIrq` before restoring IF

Sleeping mutexes and wait queues belong to NG2/NG3, when blocking call sites are
migrated to explicit sleep/wakeup.

## Expected runtime markers

With 4 vCPUs the serial log should include lines similar to:

    [SMP-NG] CPU logical=0 apic=0 legacy=0 package=0 core=0 thread=0 online=1
    [SMP-NG] CPU logical=1 apic=1 legacy=1 package=0 core=1 thread=0 online=1
    ...

Exact package/core/thread values depend on the QEMU topology.

The existing line:

    SMP4_SCHEDULER ... mode=SMP-process-affinity

is expected to remain. NG1 is a CPU-foundation milestone, not the runqueue
migration.

## Next milestones

NG2: WaitQueue + sleeping mutex infrastructure.
NG3: convert blocking Pipe/Futex/SocketPair/file-lock waits.
NG4: per-CPU runqueues and scheduler ownership in CpuLocal.
NG5: remove mandatory process home_cpu and add migration/load balancing.
NG8: address-space active CPU masks and TLB shootdown.
