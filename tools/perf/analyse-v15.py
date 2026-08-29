#!/usr/bin/env python3
from pathlib import Path
import re, statistics, sys

if len(sys.argv) != 2:
    raise SystemExit("usage: analyse-v15.py <log>")
raw = Path(sys.argv[1]).read_bytes()
for enc in ("utf-8-sig", "utf-16", "utf-16-le", "cp1252"):
    try:
        text = raw.decode(enc)
        break
    except UnicodeDecodeError:
        pass
else:
    text = raw.decode("utf-8", errors="replace")
ansi = re.compile(r"\x1b\[[0-9;]*m")
text = ansi.sub("", text)

fps = [int(x) for x in re.findall(r"\[FPS:\s*(\d+)\]", text)]
frame_perf = [tuple(map(float,m)) for m in re.findall(r"\[FRAME-PERF\].*?fps=(\d+(?:\.\d+)?).*?useful_gap_max_ms=(\d+).*?since_useful_ms=(\d+)", text)]
watch = [tuple(map(int,m)) for m in re.findall(r"\[PERF-(?:WATCHDOG|BROWSER)\].*?silence_ms=(\d+).*?frame_gap_max_ms=(\d+).*?input_to_frame_max_ms=(\d+)", text)]
online = [int(x) for x in re.findall(r"online=(\d+)", text)]
loads=[]
for line in text.splitlines():
    if "[SMP-LOAD]" not in line: continue
    vals=[int(v) for v in re.findall(r"\bc\d+=(\d+)\b", line)]
    if vals: loads.append(vals)

print("=== Bouchaud V15 performance ===")
if fps:
    print(f"FPS prefixe: n={len(fps)} mean={statistics.fmean(fps):.1f} median={statistics.median(fps):.1f} max={max(fps)}")
    act=[x for x in fps if x>0]
    if act: print(f"FPS quand actif: mean={statistics.fmean(act):.1f} max={max(act)}")
if frame_perf:
    print(f"FRAME-PERF: fps_max={max(x[0] for x in frame_perf):.1f} useful_gap_max_ms={max(x[1] for x in frame_perf):.0f} since_useful_max_ms={max(x[2] for x in frame_perf):.0f}")
if watch:
    print(f"Browser: silence_max_ms={max(x[0] for x in watch)} frame_gap_max_ms={max(x[1] for x in watch)} input_to_frame_max_ms={max(x[2] for x in watch)}")
if online:
    print(f"vCPU online observes: min={min(online)} max={max(online)}")
if loads:
    widest=max(loads, key=len)
    print(f"SMP load sample widest: {widest} ({len(widest)} CPU reportes)")

print("\nLecture: FPS=sortie utile du compositeur. Pour la fluidite navigateur, regarder aussi Browser frame_gap/input_to_frame et le nombre de vCPU online.")
