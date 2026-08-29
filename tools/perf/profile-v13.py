#!/usr/bin/env python3
from __future__ import annotations
import re, sys
from pathlib import Path

def text(path):
    raw=Path(path).read_bytes()
    if raw.startswith(b'\xff\xfe'): return raw[2:].decode('utf-16-le','replace')
    s=raw[:4096]; pairs=max(1,len(s)//2)
    if s[1::2].count(0)>pairs//3: return raw.decode('utf-16-le','replace')
    return raw.decode('utf-8-sig','replace')

def mx(pattern, data, group=1):
    vals=[int(m.group(group)) for m in re.finditer(pattern,data)]
    return max(vals) if vals else 0

def last(pattern,data):
    ms=list(re.finditer(pattern,data)); return ms[-1].groups() if ms else None

def main():
    if len(sys.argv)!=2:
        print('usage: profile-v13.py <log>'); return 2
    d=text(sys.argv[1])
    bkl=last(r'\[BKL-STATS\].*wait_ns=(\d+).*hold_ns=(\d+).*acquisitions=(\d+).*max_hold_ns=(\d+)',d)
    ww=last(r'\[WAIT-WORD\].*waits=(\d+).*signaled=(\d+).*deadlines=(\d+).*wakes=(\d+).*bucket_peak=(\d+)',d)
    tx=last(r'\[PERSIST-TXN\].*calls=(\d+).*snapshot_ns=(\d+).*hash_ns=(\d+).*io_ns=(\d+).*resume_ns=(\d+).*bytes=(\d+).*max_ns=(\d+)',d)
    ra=last(r'\[MM-READAHEAD\].*observe=(\d+).*sequential=(\d+).*requested=(\d+).*ok=(\d+).*fail=(\d+)',d)
    waitq=last(r'\[WAITQ-DETACHED\].*waits=(\d+).*legacy=(\d+).*wait_max_ns=(\d+).*depth_violations=(\d+)',d)
    print('=== Bouchaud V13 Grand Saut ===')
    print('Google search observed :', 'oui' if '/search?' in d or 'q=' in d else 'non')
    if bkl: print(f'BKL total wait/hold/acq/max : {int(bkl[0])/1e6:.1f}ms / {int(bkl[1])/1e6:.1f}ms / {bkl[2]} / {int(bkl[3])/1e6:.1f}ms')
    if ww: print('wait-word waits/signaled/deadlines/wakes/peak :',' / '.join(ww))
    if tx: print(f'persistence calls={tx[0]} snapshot={int(tx[1])/1e6:.1f}ms hash={int(tx[2])/1e6:.1f}ms io={int(tx[3])/1e6:.1f}ms resume={int(tx[4])/1e6:.1f}ms bytes={tx[5]} max={int(tx[6])/1e6:.1f}ms')
    if ra: print('readahead observe/sequential/requested/ok/fail :',' / '.join(ra))
    if waitq: print(f'waitq detached={waitq[0]} legacy={waitq[1]} max={int(waitq[2])/1e6:.1f}ms violations={waitq[3]}')
    print('max desktop watchdog :',mx(r'desktop sans heartbeat depuis (\d+) ms',d),'ms')
    print('max BKL resume wait :',mx(r'\[BKL-SCHED\].*wait_max_ns=(\d+)',d)/1e6,'ms')
    return 0
if __name__=='__main__': raise SystemExit(main())
