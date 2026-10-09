#!/usr/bin/env python3
"""Analyse robuste de la preuve GC P13 V3.

Le verdict mémoire appartient à analyse_memoire.py. Ici on valide uniquement
l'attribution des PageClient encore vivants dans le run diagnostic séparé.
Les longues lignes GC_PATH sont informatives ; le verdict de preuve repose sur
au moins deux copies identiques d'un LB:GC_SUMMARY court par jalon 10 et 20.
"""
from __future__ import annotations

import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
PAGE_STATE = re.compile(
    r"\[LB:PAGE_STATE\] pid=(\d+) seq=(\d+) roots=(\d+) created=(\d+) "
    r"detached=(\d+) finalized=(\d+) END"
)
SUMMARY = re.compile(
    r"\[LB:GC_SUMMARY\] pid=(\d+) detached=(\d+) active=(\d+) "
    r"pageclients=(\d+) retained=(\d+) rooted=(\d+) strong=(\d+) "
    r"conservative=(\d+) unknown=(\d+) layout=(\d+) timer=(\d+) "
    r"pagehost=(\d+) paths=(\d+) END"
)
PATH = re.compile(
    r"\[LB:GC_PATH\] pid=(\d+) detached=(\d+) target=([^ ]+) "
    r"root=(.*?) frame=(-?\d+) frame_label=(.*?) depth=(\d+) path=(.*?) END"
)

@dataclass(frozen=True)
class Summary:
    pid: int
    detached: int
    active: int
    pageclients: int
    retained: int
    rooted: int
    strong: int
    conservative: int
    unknown: int
    layout: int
    timer: int
    pagehost: int
    paths: int

@dataclass(frozen=True)
class PathEvidence:
    pid: int
    detached: int
    target: str
    root: str
    frame: int
    frame_label: str
    depth: int
    path: str


def analyse(text: str) -> tuple[int, list[str]]:
    clean = ANSI.sub("", text)
    states = [tuple(map(int, m.groups())) for m in PAGE_STATE.finditer(clean)]
    summaries = [Summary(*map(int, m.groups())) for m in SUMMARY.finditer(clean)]
    paths = [
        PathEvidence(
            pid=int(m[1]), detached=int(m[2]), target=m[3], root=m[4],
            frame=int(m[5]), frame_label=m[6], depth=int(m[7]), path=m[8]
        )
        for m in PATH.finditer(clean)
    ]

    if not states:
        return 2, ["GC_RETENTION_INCONCLUSIF PAGE_STATE absent"]

    max_detached_by_pid: dict[int, int] = defaultdict(int)
    for pid, _seq, _roots, _created, detached, _finalized in states:
        max_detached_by_pid[pid] = max(max_detached_by_pid[pid], detached)
    main_pid, max_detached = max(max_detached_by_pid.items(), key=lambda kv: kv[1])
    out = [f"GC_RETENTION_MAIN pid={main_pid} detached_max={max_detached}"]
    if max_detached < 20:
        return 2, out + ["GC_RETENTION_INCONCLUSIF le WebContent ouvreur n'a pas atteint 20 detachements"]

    for milestone in (10, 20):
        candidates = [s for s in summaries if s.pid == main_pid and s.detached == milestone]
        if len(candidates) < 2:
            return 2, out + [f"GC_RETENTION_INCONCLUSIF detached={milestone} resumes_complets={len(candidates)}/2"]
        counts = Counter(candidates)
        summary, copies = counts.most_common(1)[0]
        if copies < 2:
            return 2, out + [f"GC_RETENTION_INCONCLUSIF detached={milestone} resumes_incoherents={dict(counts)}"]
        if len(counts) != 1:
            return 2, out + [f"GC_RETENTION_INCONCLUSIF detached={milestone} resumes_divergents={len(counts)}"]

        expected_retained = max(0, summary.pageclients - summary.active)
        if summary.retained != expected_retained:
            return 2, out + [f"GC_RETENTION_INCONCLUSIF detached={milestone} retained={summary.retained} attendu={expected_retained}"]
        if summary.strong + summary.conservative + summary.unknown != summary.pageclients:
            return 2, out + [f"GC_RETENTION_INCONCLUSIF detached={milestone} partition_racines_invalide"]
        if summary.rooted != summary.pageclients - summary.unknown:
            return 2, out + [f"GC_RETENTION_INCONCLUSIF detached={milestone} rooted={summary.rooted} incoherent"]
        if summary.layout + summary.timer + summary.pagehost > summary.strong:
            return 2, out + [f"GC_RETENTION_INCONCLUSIF detached={milestone} sous_categories_strong_incoherentes"]
        if summary.unknown != 0:
            return 2, out + [f"GC_RETENTION_INCONCLUSIF detached={milestone} unknown={summary.unknown}"]

        out.append(
            "GC_RETENTION_JALON "
            f"pid={main_pid} detached={milestone} copies={copies} active={summary.active} "
            f"pageclients={summary.pageclients} retained={summary.retained} rooted={summary.rooted} "
            f"strong={summary.strong} conservative={summary.conservative} "
            f"layout={summary.layout} timer={summary.timer} pagehost={summary.pagehost} paths={summary.paths}"
        )

        shown = 0
        seen: set[tuple[str, str, str]] = set()
        for p in paths:
            if p.pid != main_pid or p.detached != milestone:
                continue
            sig = (p.root, p.frame_label, p.path)
            if sig in seen:
                continue
            seen.add(sig)
            out.append(
                f"GC_RETENTION_PATH detached={milestone} root={p.root} frame={p.frame} "
                f"frame_label={p.frame_label or '<none>'} depth={p.depth} path={p.path}"
            )
            shown += 1
            if shown >= 5:
                break

    out.append("GC_RETENTION_PREUVE_OK resumes_redondants=2plus unknown=0")
    return 0, out


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: analyse_gc_retention.py <serie-gc-retention.log>", file=sys.stderr)
        return 2
    code, lines = analyse(Path(sys.argv[1]).read_text(errors="replace"))
    print("\n".join(lines))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
