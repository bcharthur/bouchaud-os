#!/usr/bin/env python3
from __future__ import annotations
import re, sys
from pathlib import Path

def read_log(path: Path) -> str:
    raw = path.read_bytes()
    if raw.startswith(b"\xff\xfe"):
        return raw[2:].decode("utf-16-le", errors="replace")
    if raw.startswith(b"\xfe\xff"):
        return raw[2:].decode("utf-16-be", errors="replace")
    sample = raw[:4096]
    pairs = max(1, len(sample)//2)
    if sample[1::2].count(0) > pairs//3 and sample[0::2].count(0) < pairs//10:
        return raw.decode("utf-16-le", errors="replace")
    return raw.decode("utf-8-sig", errors="replace")

RE_PROV = re.compile(r"\[SMP-PROV\].*held=(\d+)ms.*depth=(\d+).*kind=(\d+).*task=(\d+).*syscall=([0-9]+):([0-9]+)")
RE_WAITQ = re.compile(r"\[WAITQ-DETACHED\] waits=(\d+) legacy=(\d+) wait_ns=(\d+) wait_max_ns=(\d+) schedule_loops=(\d+) depth_violations=(\d+)")
RE_BKL = re.compile(r"\[BKL-COMPTES\].*reprise_max_ns=(\d+).*parks=(\d+).*wake_ipis=(\d+).*reveils_sans_acq=(\d+)")
RE_WATCH = re.compile(r"desktop sans heartbeat depuis (\d+) ms")

def main():
    if len(sys.argv) != 2:
        print("usage: analyse-final-v12.py <log>")
        return 2
    text = read_log(Path(sys.argv[1]))
    stalls = text.count("[SMP-STALL]")
    max_hold = max_kernel = max_user = heartbeat = 0
    last_waitq = last_bkl = worst = None

    for line in text.splitlines():
        if m := RE_PROV.search(line):
            held, depth, kind, task, nr, phase = map(int, m.groups())
            max_hold = max(max_hold, held)
            if task == 0: max_kernel = max(max_kernel, held)
            else: max_user = max(max_user, held)
            if worst is None or held > worst[0]:
                worst = (held, depth, kind, task, nr, phase, line)
        if m := RE_WAITQ.search(line):
            last_waitq = tuple(map(int, m.groups()))
        if m := RE_BKL.search(line):
            last_bkl = tuple(map(int, m.groups()))
        if m := RE_WATCH.search(line):
            heartbeat = max(heartbeat, int(m.group(1)))

    print("=== Bouchaud OS Final V12 ===")
    print(f"SMP-STALL               : {stalls}")
    print(f"max BKL hold            : {max_hold} ms")
    print(f"max hold kernel/task0   : {max_kernel} ms")
    print(f"max hold user           : {max_user} ms")
    print(f"desktop heartbeat max   : {heartbeat} ms")
    if last_waitq:
        waits, legacy, total, mx, loops, violations = last_waitq
        print(f"waitq detached/legacy   : {waits}/{legacy}")
        print(f"waitq detached max      : {mx/1e6:.3f} ms")
        print(f"waitq schedule loops    : {loops}")
        print(f"waitq depth violations  : {violations}")
    else:
        print("WAITQ-DETACHED          : absent")
    if last_bkl:
        resume, parks, ipis, useless = last_bkl
        print(f"resume max              : {resume/1e6:.3f} ms")
        print(f"parks/ipis/useless      : {parks}/{ipis}/{useless}")
    if worst:
        print("worst:", worst[-1])

    print("verdict:")
    if last_waitq and last_waitq[-1]:
        print("- ECHEC invariant: detached wait returned with BKL depth != 0")
    elif max_hold < 500 and heartbeat < 2000:
        print("- cible atteinte sur ce run")
    elif max_kernel >= 1000:
        print("- longue tenue restante = kernel task; ne plus accuser poll par contexte CPU stale")
    elif max_user >= 1000:
        print("- longue tenue user restante; utiliser syscall/phase du worst")
    else:
        print("- amélioration partielle; comparer au V11C")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
