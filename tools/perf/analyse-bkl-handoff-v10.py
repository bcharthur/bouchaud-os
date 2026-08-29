#!/usr/bin/env python3
"""Analyse compacte du BKL Waiter Handoff V10."""

from __future__ import annotations
import re
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")

def last_matching(lines, token):
    for line in reversed(lines):
        if token in line:
            return line
    return None

def kv(line):
    if not line:
        return {}
    clean = ANSI.sub("", line)
    return {k: int(v, 0) for k, v in re.findall(
        r"([A-Za-z_]+)=((?:0x)?[0-9a-fA-F]+)", clean
    )}

def main():
    if len(sys.argv) != 2:
        print("usage: analyse-bkl-handoff-v10.py <log>")
        return 2

    p = Path(sys.argv[1])
    lines = p.read_text(encoding="utf-8", errors="replace").splitlines()

    h = kv(last_matching(lines, "[BKL-HANDOFF]"))
    b = kv(last_matching(lines, "[BKL-COMPTES]"))
    s = kv(last_matching(lines, "[BKL-SCHED]"))
    k = kv(last_matching(lines, "[KTHREAD-BKL]"))

    if not h:
        print("Aucune ligne [BKL-HANDOFF] : V10 absent ou rapport non atteint.")
        return 3

    prepared = h.get("prepared", 0)
    claims = h.get("claims", 0)
    wakeups = h.get("wakes", 0)
    deferrals = h.get("deferrals", 0)
    rollbacks = h.get("rollbacks", 0)
    expired = h.get("expired", 0)
    claim_max = h.get("claim_wait_max_ns", 0)

    parks = b.get("parks", 0)
    wake_ipis = b.get("wake_ipis", 0)
    useless = b.get("reveils_sans_acq", 0)
    reprise_max = b.get("reprise_max_ns", s.get("wait_max_ns", 0))

    print("=== Bouchaud BKL Handoff V10 ===")
    print(f"prepared              : {prepared}")
    print(f"claims                : {claims}")
    print(f"handoff wakeups       : {wakeups}")
    print(f"new-entry deferrals   : {deferrals}")
    print(f"race rollbacks        : {rollbacks}")
    print(f"lease expirations     : {expired}")
    print(f"claim wait max        : {claim_max/1e6:.3f} ms")
    print(f"parks                 : {parks}")
    print(f"wake IPIs             : {wake_ipis}")
    print(f"wakes sans acquisition: {useless}")
    print(f"resume max            : {reprise_max/1e6:.3f} ms")
    if k:
        print(f"desktop BKL gap max   : {k.get('gap_max_ns',0)/1e6:.3f} ms")

    if wake_ipis:
        print(f"ineffective/wake IPI  : {100.0*useless/wake_ipis:.1f}%")

    print("\nLecture:")
    if prepared and claims * 2 >= prepared:
        print("- Le handoff est effectivement consommé par les waiters sélectionnés.")
    elif prepared:
        print("- Beaucoup de handoffs restent non claimés: regarder expirations et resume_cancel.")
    if expired > max(10, prepared // 20):
        print("- Trop d'expirations: la lease ou le scheduling mérite un audit.")
    if useless and wake_ipis and useless > wake_ipis:
        print("- Le churn park/wake reste élevé; comparer surtout avec le run V9.1.")
    if reprise_max >= 500_000_000:
        print("- La queue BKL conserve une longue traîne (>500 ms).")
    elif reprise_max:
        print("- La reprise scheduler reste sous 500 ms sur le dernier maximum observé.")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
