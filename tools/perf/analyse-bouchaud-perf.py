#!/usr/bin/env python3
"""Analyse un log série Bouchaud OS et classe les goulots navigateur.

Usage:
    python tools/perf/analyse-bouchaud-perf.py bkl-v4-google.log
"""

from __future__ import annotations
import re
import sys
from pathlib import Path
from collections import defaultdict

RE_PROC = re.compile(r"\[PROC-SAMPLE\].*pid=(\d+) name=([^ ]+) cpu_pct=(\d+)")
RE_GUI = re.compile(r"\[gui\] client pid=(\d+).*silence (\d+) ms")
RE_BKL = re.compile(r"\[BKL-COMPTES\].*reprise_max_ns=(\d+).*parks=(\d+).*wake_ipis=(\d+).*reveils_sans_acq=(\d+)")
RE_HEALTH = re.compile(r"\[BKL-HEALTH\].*owner_depth_ok=(\d+).*resume_oldest_ns=(\d+)")
RE_PF = re.compile(r"\[MM-NG6\].*fault_resolved=(\d+)")
RE_GUI_PRESENT = re.compile(r"\[GUI-PRESENT\].*since_last_present_ms=(\d+)")
RE_PERF = re.compile(
    r"\[PERF-BROWSER\].*silence_ms=(\d+).*frame_gap_max_ms=(\d+).*input_to_frame_max_ms=(\d+).*pf_delta=(\d+).*bottleneck=([^\s]+)"
)
RE_WD = re.compile(r"\[PERF-WATCHDOG\].*bottleneck=([^\s]+).*pf_delta=(\d+)")
RE_PANIC = re.compile(r"\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|GENERAL PROTECTION|page fault @")

def main() -> int:
    if len(sys.argv) != 2:
        print("usage: analyse-bouchaud-perf.py <log>")
        return 2
    path = Path(sys.argv[1])
    text = path.read_text(encoding="utf-8", errors="replace")
    lines = text.splitlines()

    cpu_max = defaultdict(int)
    gui_silence_max = 0
    bkl_resume_max = 0
    bkl_parks = bkl_wakes = bkl_wasted = 0
    bkl_invariant_bad = False
    resume_oldest_max = 0
    pf_first = pf_last = None
    gui_present_gap_max = 0
    perf_silence = perf_frame_gap = perf_input_latency = perf_pf_delta = 0
    bottlenecks = defaultdict(int)
    panics = []

    for idx, line in enumerate(lines, 1):
        if m := RE_PROC.search(line):
            cpu_max[m.group(2)] = max(cpu_max[m.group(2)], int(m.group(3)))
        if m := RE_GUI.search(line):
            gui_silence_max = max(gui_silence_max, int(m.group(2)))
        if m := RE_BKL.search(line):
            bkl_resume_max = max(bkl_resume_max, int(m.group(1)))
            bkl_parks = max(bkl_parks, int(m.group(2)))
            bkl_wakes = max(bkl_wakes, int(m.group(3)))
            bkl_wasted = max(bkl_wasted, int(m.group(4)))
        if m := RE_HEALTH.search(line):
            bkl_invariant_bad |= m.group(1) != "1"
            resume_oldest_max = max(resume_oldest_max, int(m.group(2)))
        if m := RE_PF.search(line):
            v = int(m.group(1))
            pf_first = v if pf_first is None else pf_first
            pf_last = v
        if m := RE_GUI_PRESENT.search(line):
            gui_present_gap_max = max(gui_present_gap_max, int(m.group(1)))
        if m := RE_PERF.search(line):
            perf_silence = max(perf_silence, int(m.group(1)))
            perf_frame_gap = max(perf_frame_gap, int(m.group(2)))
            perf_input_latency = max(perf_input_latency, int(m.group(3)))
            perf_pf_delta = max(perf_pf_delta, int(m.group(4)))
            bottlenecks[m.group(5)] += 1
        if m := RE_WD.search(line):
            bottlenecks[m.group(1)] += 1
            perf_pf_delta = max(perf_pf_delta, int(m.group(2)))
        if RE_PANIC.search(line):
            panics.append((idx, line))

    pf_growth = 0 if pf_first is None or pf_last is None else pf_last - pf_first
    wc_cpu = max((v for k, v in cpu_max.items() if "WebContent" in k), default=0)
    comp_cpu = max((v for k, v in cpu_max.items() if "Compositor" in k), default=0)

    # Classification de repli pour les anciens logs sans PERF-BROWSER.
    if bottlenecks:
        likely = max(bottlenecks, key=bottlenecks.get)
    elif bkl_invariant_bad or resume_oldest_max >= 50_000_000:
        likely = "kernel-bkl"
    elif pf_growth >= 5_000:
        likely = "memory-pagefault"
    elif wc_cpu >= 90 and gui_silence_max >= 500:
        likely = "browser-renderer"
    elif gui_present_gap_max >= 500:
        likely = "gui-present"
    else:
        likely = "mixed/insufficient-data"

    print("=== Bouchaud Performance Observatory ===")
    print(f"log                    : {path}")
    print(f"bottleneck probable    : {likely}")
    print(f"WebContent CPU max     : {wc_cpu}%")
    print(f"Compositor CPU max     : {comp_cpu}%")
    print(f"silence client GUI max : {max(gui_silence_max, perf_silence)} ms")
    print(f"frame gap max          : {perf_frame_gap or gui_present_gap_max} ms")
    print(f"input->frame max       : {perf_input_latency} ms")
    print(f"faults progression     : {pf_growth}")
    print(f"PF delta perf max      : {perf_pf_delta}")
    print(f"BKL resume max         : {bkl_resume_max / 1_000_000:.2f} ms")
    print(f"BKL resume oldest max  : {resume_oldest_max / 1_000_000:.2f} ms")
    print(f"BKL parks/wakes/wasted : {bkl_parks}/{bkl_wakes}/{bkl_wasted}")
    print(f"BKL invariant bad      : {bkl_invariant_bad}")
    print(f"panic/crash markers    : {len(panics)}")

    if bottlenecks:
        print("classifications runtime:")
        for name, count in sorted(bottlenecks.items(), key=lambda kv: (-kv[1], kv[0])):
            print(f"  {name:24s} {count}")

    if panics:
        print("\nDerniers marqueurs crash:")
        for idx, line in panics[-5:]:
            print(f"  L{idx}: {line}")

    return 0

if __name__ == "__main__":
    raise SystemExit(main())
