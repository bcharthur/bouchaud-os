#!/usr/bin/env python3
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
errors = []

def text(path: str) -> str:
    p = ROOT / path
    if not p.is_file():
        errors.append(f"missing: {path}")
        return ""
    raw = p.read_bytes()
    if raw.startswith(b"\xef\xbb\xbf"):
        errors.append(f"UTF-8 BOM forbidden: {path}")
    if b"\r\n" in raw:
        errors.append(f"CRLF forbidden in P0-NG1 source: {path}")
    return raw.decode("utf-8")

checks = {
    "src/kernel/sync/mod.rs": ["pub mod lockdep;", "RankedSpinLock"],
    "src/kernel/sync/lockdep.rs": ["enum LockClass", "[LOCKDEP]", "before_acquire"],
    "src/kernel/sync/ranked.rs": ["preempt::disable", "lockdep::acquired", "preempt::enable"],
    "src/kernel/scheduler/core.rs": ["pub mod latency;", "pub mod preempt;"],
    "src/kernel/scheduler/preempt.rs": ["safe_point", "need_resched", "[SCHED-NG-PREEMPT]"],
    "src/kernel/scheduler/latency.rs": ["interactive_max_ns", "[SCHED-NG-LAT]"],
    "src/arch/x86_64/idt/timer.rs": ["scheduler::preempt::request_local"],
    "src/arch/x86_64/usermode.rs": ["scheduler::preempt::safe_point"],
    "src/kernel/process/thread/tache.rs": ["ready_since_ns"],
    "src/kernel/process/thread/creation.rs": ["ready_since_ns", "preempt::request_cpu"],
    # `ready_since_ns` est devenu atomique : la remise a zero s'ecrit
    # `range(0)`. Ce que la regle protege est inchange -- la latence
    # ready-to-run doit etre consommee au moment ou la tache prend le CPU,
    # sans quoi la mesure suivante la compterait deux fois.
    "src/kernel/process/thread/commutation.rs": ["scheduler::latency::record", "ready_since_ns.range(0)"],
    "src/kernel/memory/heap.rs": ["struct NgHeap", "CLASS_SIZES", "[MEM-NG-HEAP]"],
    "src/kernel/memory/frame_cache.rs": ["LOCAL_CAPACITY", "free_frame_global", "[MEM-NG-FRAMECACHE]"],
    "src/kernel/memory/pressure.rs": ["enum Level", "reclaim_now", "note_oom", "[MEM-NG-PRESSURE]"],
    "src/kernel/memory/page_cache.rs": ["reclaim_pages", "pressure_target", "frame_cache::alloc_frame"],
    "src/compat/linux/verrous.rs": ["RankedSpinLock", "LockClass::PosixRecord"],
    "src/kernel/process/process.rs": ["RankedSpinLock", "LockClass::ProcessTable"],
}

for path, needles in checks.items():
    s = text(path)
    for needle in needles:
        if needle not in s:
            errors.append(f"{path}: missing marker {needle!r}")

for path, forbidden in (("src/compat/linux/verrous.rs", "static mut VERROUS"), ("src/kernel/process/process.rs", "static mut TABLE")):
    s = text(path)
    if forbidden in s:
        errors.append(f"{path}: legacy mutable global reintroduced")

heap = text("src/kernel/memory/heap.rs")
if "LockedHeap" not in heap or "CACHE_READY" not in heap:
    errors.append("heap-ng must retain proven backing allocator and gate local caches")

page = text("src/kernel/memory/page_cache.rs")
if "free_frame_global" not in page:
    errors.append("pressure reclaim must bypass local cache and return frames globally")

user = text("src/arch/x86_64/usermode.rs")
pos_drop = user.find("drop(kernel);")
pos_safe = user.find("scheduler::preempt::safe_point")
if pos_drop < 0 or pos_safe < 0 or pos_safe < pos_drop:
    errors.append("kernel safe-point must occur after outer BKL drop")

# Ownership invariant: CURRENT_IS_KERNEL accessor belongs to thread/metriques.rs.
# tache.rs only defines Task layout. Duplicating the function in two include!
# fragments creates E0428 in the common kernel::task module.
tache = text("src/kernel/process/thread/tache.rs")
metriques_path = ROOT / "src/kernel/process/thread/metriques.rs"
if not metriques_path.is_file():
    errors.append("missing: src/kernel/process/thread/metriques.rs")
    metriques = ""
else:
    # Base file: tolerate the checkout's Windows CRLF policy. Only bundle-owned
    # source files are required to be LF by this validator.
    metriques = metriques_path.read_text(encoding="utf-8", errors="replace")
needle = "pub fn current_is_kernel_task()"
if tache.count(needle) != 0:
    errors.append("thread/tache.rs must not define current_is_kernel_task (owned by metriques.rs)")
if metriques.count(needle) != 1:
    errors.append("thread/metriques.rs must define current_is_kernel_task exactly once")

# Rust module resolution invariant for #[path = "scheduler/core.rs"]:
# latency.rs and preempt.rs live directly in scheduler/, not scheduler/core/.
for required in ("src/kernel/scheduler/latency.rs", "src/kernel/scheduler/preempt.rs"):
    if not (ROOT / required).is_file():
        errors.append(f"scheduler module misplaced or missing: {required}")

if errors:
    print("P0_NG1_FAILED")
    for err in errors:
        print(f"  - {err}")
    sys.exit(1)
print("P0_NG1_OK")
