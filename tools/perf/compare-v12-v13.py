#!/usr/bin/env python3
import re, sys
from pathlib import Path

def read(p):
    b=Path(p).read_bytes()
    if b.startswith(b'\xff\xfe') or b[:4096][1::2].count(0)>max(1,len(b[:4096])//6): return b.decode('utf-16-le','replace')
    return b.decode('utf-8-sig','replace')
def lastnum(d,pat):
    m=list(re.finditer(pat,d)); return int(m[-1].group(1)) if m else 0
def metrics(d):
    return {
      'bkl_max_ms': lastnum(d,r'\[BKL-STATS\].*max_hold_ns=(\d+)')/1e6,
      'waitq_bkl_ms': lastnum(d,r'waitq_bkl_wait_ns=(\d+)')/1e6,
      'faults': lastnum(d,r'fault_resolved=(\d+)'),
      'presents': lastnum(d,r'present_calls=(\d+)'),
      'watchdog_ms': max([int(x) for x in re.findall(r'desktop sans heartbeat depuis (\d+) ms',d)] or [0]),
    }
if len(sys.argv)!=3:
    print('usage: compare-v12-v13.py <before.log> <v13.log>'); raise SystemExit(2)
a,b=metrics(read(sys.argv[1])),metrics(read(sys.argv[2]))
print('metric                 before        v13       delta')
for k in a:
    x,y=a[k],b[k]; print(f'{k:20} {x:12.2f} {y:12.2f} {y-x:12.2f}')
