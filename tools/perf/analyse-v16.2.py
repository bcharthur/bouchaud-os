#!/usr/bin/env python3
import re, sys
from pathlib import Path

if len(sys.argv) != 2:
    print("usage: python tools/perf/analyse-v16.2.py <log>")
    raise SystemExit(2)

text = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")

def numbers(pattern):
    return [int(x) for x in re.findall(pattern, text)]

bkl = numbers(r"\[BKL-MAX-HOLD\]\s+ns=(\d+)")
gaps = numbers(r"frame_gap_max_ms=(\d+)")
silences = numbers(r"silence_ms=(\d+)")
fps = numbers(r"\[FPS:\s*(\d+)\]")
zero = re.findall(r"\[MM-ZERO-CLUSTER\].*?faults=(\d+).*?triggered=(\d+).*?mapped=(\d+).*?max_batch=(\d+)", text)
cluster = re.findall(r"\[MM-CLUSTER\].*?attempts=(\d+).*?mapped=(\d+).*?max_batch=(\d+)", text)
panic_zombie = "zombie task resumed after exec quiescence" in text
panic_any = "*** KERNEL PANIC ***" in text

print("=== Bouchaud V16.2 ===")
print(f"panic                    : {'OUI' if panic_any else 'non'}")
print(f"panic exec-zombie        : {'OUI' if panic_zombie else 'non'}")
print(f"BKL max                  : {(max(bkl)/1e6 if bkl else 0):.1f} ms")
print(f"browser silence max      : {max(silences) if silences else 0} ms")
print(f"frame gap max            : {max(gaps) if gaps else 0} ms")
if fps:
    active = [v for v in fps if v > 0]
    print(f"FPS max                  : {max(fps)}")
    print(f"FPS actifs moyen         : {(sum(active)/len(active) if active else 0):.1f}")
if cluster:
    a,m,b = map(int, cluster[-1])
    print(f"file cluster final       : attempts={a} mapped={m} max_batch={b}")
if zero:
    f,t,m,b = map(int, zero[-1])
    print(f"zero cluster final       : faults={f} triggered={t} mapped={m} max_batch={b}")

print()
if panic_zombie:
    print("ECHEC P0: la retraite exec-zombie n'est pas corrigee.")
elif bkl and max(bkl) > 1_000_000_000:
    print("P0 perf restant: BKL > 1 s. Inspecter le nouveau site_acquisition/site_tenue.")
elif bkl and max(bkl) > 200_000_000:
    print("BKL encore trop long (>200 ms), mais plus de gel multi-seconde si le gap suit.")
else:
    print("BKL: pas de tenue >200 ms observee dans ce log.")
