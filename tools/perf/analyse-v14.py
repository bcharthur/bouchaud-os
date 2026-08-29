#!/usr/bin/env python3
from __future__ import annotations
import re, sys
from pathlib import Path

if len(sys.argv) != 2:
    raise SystemExit("usage: analyse-v14.py <log>")
p=Path(sys.argv[1]); raw=p.read_bytes()
for enc in ("utf-8-sig","utf-16-le","utf-16-be"):
    try:
        text=raw.decode(enc)
        if "[kernel]" in text or "[BKL-" in text or "[SMP-" in text: break
    except UnicodeDecodeError: pass
else: text=raw.decode("utf-8",errors="replace")
ansi=re.compile(r"\x1b\[[0-9;]*m")
text=ansi.sub("",text)

def last(pattern):
    ms=list(re.finditer(pattern,text))
    return ms[-1].groupdict() if ms else None

def imax(pattern, key):
    vals=[int(m.groupdict()[key]) for m in re.finditer(pattern,text)]
    return max(vals) if vals else None

bkl=last(r"\[BKL-STATS\].*?wait_ns=(?P<wait>\d+).*?hold_ns=(?P<hold>\d+).*?acquisitions=(?P<acq>\d+).*?max_hold_ns=(?P<max>\d+).*?parks=(?P<parks>\d+).*?wake_ipis=(?P<wakes>\d+)")
back=last(r"\[BACKING-CACHE\] reads=(?P<reads>\d+) bytes=(?P<bytes>\d+).*?clean_hit=(?P<hit>\d+) clean_miss=(?P<miss>\d+)")
mm=last(r"\[MM-NG6\] fault_resolved=(?P<faults>\d+).*?ata_acquires=(?P<ata>\d+) ata_wait_ns=(?P<atawait>\d+)")
cluster=last(r"\[MM-CLUSTER\] attempts=(?P<attempts>\d+) mapped=(?P<mapped>\d+) cache_miss=(?P<miss>\d+) already=(?P<already>\d+) aborts=(?P<aborts>\d+) max_batch=(?P<batch>\d+)")
ra=last(r"\[MM-READAHEAD\] observe=(?P<observe>\d+) sequential=(?P<seq>\d+) requested=(?P<req>\d+) ok=(?P<ok>\d+) fail=(?P<fail>\d+).*?max_window=(?P<window>\d+)")
watch_silence=imax(r"\[PERF-WATCHDOG\].*?silence_ms=(?P<v>\d+)","v")
frame_gap=imax(r"\[PERF-(?:WATCHDOG|BROWSER)\].*?frame_gap_max_ms=(?P<v>\d+)","v")
input_gap=imax(r"\[PERF-(?:WATCHDOG|BROWSER)\].*?input_to_frame_max_ms=(?P<v>\d+)","v")

print("=== Bouchaud OS V14 performance ===")
if bkl:
    print(f"BKL: max={int(bkl['max'])/1e6:.2f} ms hold={int(bkl['hold'])/1e9:.3f}s wait={int(bkl['wait'])/1e9:.3f}s acq={bkl['acq']} parks={bkl['parks']} wake_ipis={bkl['wakes']}")
if back:
    reads=int(back['reads']); bs=int(back['bytes'])
    print(f"Backing: reads={reads} bytes={bs} avg_read={(bs/reads/1024 if reads else 0):.1f} KiB clean_hit={back['hit']} clean_miss={back['miss']}")
if mm: print(f"MM: faults_resolved={mm['faults']} ata_acquires={mm['ata']} ata_wait={int(mm['atawait'])/1e6:.1f} ms")
if cluster: print(f"Cluster: mapped={cluster['mapped']}/{cluster['attempts']} cache_miss={cluster['miss']} already={cluster['already']} aborts={cluster['aborts']} max_batch={cluster['batch']}")
if ra: print(f"Readahead: observed={ra['observe']} sequential={ra['seq']} requested={ra['req']} ok={ra['ok']} fail={ra['fail']} max_window={ra['window']}")
print(f"UX: max_silence={watch_silence if watch_silence is not None else 'n/a'} ms frame_gap_max={frame_gap if frame_gap is not None else 'n/a'} ms input_to_frame_max={input_gap if input_gap is not None else 'n/a'} ms")

print("\nTargets V14 (steady Google, not a guarantee under host overload):")
print("- no WAITQ depth violation")
print("- WRITE no longer appears as an outer-BKL site=212 max hold")
print("- BKL max hold preferably <100 ms")
print("- backing average read materially above V13 (~16 KiB baseline)")
print("- MM-CLUSTER mapped >0 and fault growth slows after warm-up")
print("- frame gap trends below 500 ms; WHPX1 is the UX profile")
