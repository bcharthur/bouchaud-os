#!/usr/bin/env python3
"""C22 : comparer les VMA anonymes avant M1 et M2 (inventaire noyau)."""
import argparse
import re
from pathlib import Path

BEGIN = re.compile(r"\[C22:VMA-BEGIN\] t=(\d+) pid=(\d+) total=(\d+)")
ITEM = re.compile(r"\[C22:VMA\] t=(\d+) pid=(\d+) id=(\d+) start=(0x[0-9a-f]+) end=(0x[0-9a-f]+) size_kio=(\d+) type=zero")
END = re.compile(r"\[C22:VMA-END\] t=(\d+) pid=(\d+)")
MARK = re.compile(r"HOST_MEMOIRE_REPERE m=([012]) n=(\d+)")

def compare(log: str, pid=20):
    snapshots = []
    markers = {}
    current = None
    for index, line in enumerate(log.splitlines()):
        if match := BEGIN.search(line):
            if int(match[2]) == pid:
                current = (index, int(match[1]), int(match[3]), {})
        elif match := ITEM.search(line):
            if current and int(match[2]) == pid and int(match[1]) == current[1]:
                current[3][int(match[3])] = (int(match[4], 16), int(match[5], 16), int(match[6]))
        elif match := END.search(line):
            if current and int(match[2]) == pid and int(match[1]) == current[1]:
                snapshots.append((index, current[1], current[2], current[3]))
                current = None
        if match := MARK.search(line):
            markers.setdefault(int(match[1]), index)
    if not 1 in markers or not 2 in markers:
        raise ValueError("reperes M1/M2 absents")
    before = []
    for phase in (1, 2):
        candidates = [v for v in snapshots if v[0] < markers[phase]]
        if not candidates:
            raise ValueError(f"aucun inventaire VMA complet avant M{phase}")
        before.append(candidates[-1])
    m1, m2 = before
    added = {k:v for k,v in m2[3].items() if k not in m1[3]}
    removed = {k:v for k,v in m1[3].items() if k not in m2[3]}
    return m1, m2, added, removed

def main():
    ap=argparse.ArgumentParser()
    ap.add_argument("journal")
    ap.add_argument("--label", default="essai")
    a=ap.parse_args()
    try:
        m1, m2, added, removed = compare(Path(a.journal).read_text(errors="replace"))
    except ValueError as exc:
        raise SystemExit(f"C22_VMA_INCONCLUSIF {exc}")
    print(f"C22_VMA_DIFF cas={a.label} total_M1={m1[2]} total_M2={m2[2]} nouveaux={len(added)} disparus={len(removed)}")
    for label, rows in (("NOUVEAU", added), ("DISPARU", removed)):
        for id, (start, end, kib) in sorted(rows.items()):
            print(f"C22_VMA_{label} cas={a.label} id={id} start={start:#x} end={end:#x} size_kio={kib}")
    print("C22_VMA_DIFF_OK")
if __name__=="__main__":
    main()
