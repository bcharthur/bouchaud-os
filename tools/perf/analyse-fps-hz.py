#!/usr/bin/env python3
from __future__ import annotations
import re, statistics, sys
from pathlib import Path

ANSI = re.compile(r'\x1b\[[0-9;]*m')
PREFIX = re.compile(r'\[(\d{2}:\d{2}:\d{2})\].*?\[FPS:\s*(--|\d+)\]\[Hz:\s*(--|\d+)\]')
TARGET = re.compile(r'\[FRAME-CLOCK\].*?target_hz=(\d+)\.(\d+)')
FIRST = ('M11_FIRST_FRAME', 'PERF_FIRST_PAINT')
GAP = re.compile(r'frame_gap_max_ms=(\d+)')
INPUT_GAP = re.compile(r'input_to_frame_max_ms=(\d+)')

def read(path: Path) -> str:
    raw = path.read_bytes()
    if raw.startswith(b'\xff\xfe'):
        return raw[2:].decode('utf-16-le', errors='replace')
    sample = raw[:4096]
    if sample and sample[1::2].count(0) > max(8, len(sample)//8):
        return raw.decode('utf-16-le', errors='replace')
    return raw.decode('utf-8-sig', errors='replace')

def percentile(values, q):
    if not values:
        return 0.0
    s=sorted(values)
    i=(len(s)-1)*q
    lo=int(i); hi=min(lo+1,len(s)-1); f=i-lo
    return s[lo]*(1-f)+s[hi]*f

def main():
    if len(sys.argv) != 2:
        print('usage: analyse-fps-hz.py <log>')
        return 2
    lines=ANSI.sub('', read(Path(sys.argv[1]))).splitlines()
    started=False
    per_second={}
    target=None
    max_gap=max_input=0
    for line in lines:
        if any(marker in line for marker in FIRST):
            started=True
        if m:=TARGET.search(line):
            target=int(m.group(1))+int(m.group(2))/10
        if m:=GAP.search(line): max_gap=max(max_gap,int(m.group(1)))
        if m:=INPUT_GAP.search(line): max_input=max(max_input,int(m.group(1)))
        m=PREFIX.search(line)
        if not m or not started or m.group(2)=='--' or m.group(3)=='--':
            continue
        # Une valeur par seconde évite de surpondérer les périodes bavardes du journal.
        per_second[m.group(1)] = (int(m.group(2)), int(m.group(3)))
    samples=list(per_second.values())
    if not samples:
        print('Aucun échantillon FPS/Hz après la première trame.')
        return 1
    fps=[x[0] for x in samples]; hz=[x[1] for x in samples]
    print('=== Bouchaud FPS/Hz ===')
    print(f'échantillons        : {len(samples)} seconde(s)')
    if target is not None: print(f'cible compositeur   : {target:.1f} Hz')
    print(f'FPS moyen / médiane : {statistics.mean(fps):.1f} / {statistics.median(fps):.1f}')
    print(f'FPS p5 / p1 / max   : {percentile(fps,.05):.1f} / {percentile(fps,.01):.1f} / {max(fps)}')
    print(f'Hz moyen / médiane  : {statistics.mean(hz):.1f} / {statistics.median(hz):.1f}')
    print(f'Hz p5 / max         : {percentile(hz,.05):.1f} / {max(hz)}')
    for threshold in (30,60,90,120):
        pct=sum(v>=threshold for v in fps)*100/len(fps)
        print(f'FPS >= {threshold:<3}       : {pct:5.1f}%')
    print(f'frame gap max       : {max_gap} ms')
    print(f'input->frame max    : {max_input} ms')
    print('note: FPS=trames utiles; Hz=cadence effective du compositeur, pas refresh physique QEMU.')
    return 0

if __name__=='__main__':
    raise SystemExit(main())
