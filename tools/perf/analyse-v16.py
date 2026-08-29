#!/usr/bin/env python3
import re, sys, statistics
from pathlib import Path
if len(sys.argv)!=2: raise SystemExit("usage: analyse-v16.py <log>")
raw=Path(sys.argv[1]).read_bytes()
for enc in ("utf-8-sig","utf-16","utf-16-le"):
    try: text=raw.decode(enc); break
    except UnicodeError: pass
else: text=raw.decode("utf-8",errors="replace")
fps=[int(x) for x in re.findall(r"\[FPS:\s*(\d+)\]",text)]
maxb=[int(x) for x in re.findall(r"\[BKL-MAX-HOLD\] ns=(\d+)",text)]
pf=[int(x) for x in re.findall(r"pf_delta=(\d+)",text)]
sil=[int(x) for x in re.findall(r"silence_ms=(\d+)",text)]
gaps=[int(x) for x in re.findall(r"(?:frame_gap_max_ms|useful_gap_max_ms)=(\d+)",text)]
zero=re.findall(r"\[MM-ZERO-CLUSTER\] faults=(\d+) triggered=(\d+) mapped=(\d+) already=(\d+) aborts=(\d+) max_batch=(\d+)",text)
iface=re.findall(r"\[INTERFACE-WAIT\].*?detached=(\d+).*?depth1=(\d+).*?nested=(\d+).*?max_depth=(\d+).*?depth_violations=(\d+)",text)
print("Bouchaud V16 analysis")
if fps: print(f"FPS samples={len(fps)} median={statistics.median(fps):.1f} max={max(fps)} zero_pct={100*sum(v==0 for v in fps)/len(fps):.1f}%")
if maxb: print(f"BKL max={max(maxb)/1e6:.1f} ms")
if pf: print(f"PF delta max/window={max(pf)}")
if sil: print(f"Browser silence max={max(sil)} ms")
if gaps: print(f"Frame/useful gap max={max(gaps)} ms")
if zero:
    f,t,m,a,ab,mb=map(int,zero[-1]); print(f"Zero cluster faults={f} triggered={t} mapped={m} already={a} aborts={ab} max_batch={mb}")
if iface:
    d,d1,n,md,v=map(int,iface[-1]); print(f"Interface detached={d} depth1={d1} nested={n} max_depth={md} violations={v}")
print("PASS depth contract" if not iface or int(iface[-1][-1])==0 else "FAIL depth contract")
