#!/usr/bin/env python3
"""Valide et classe la preuve BOUCHAUD_P13_GC_RETENTION_V2.

Cette analyse ne declare jamais la memoire bornee. Elle attribue uniquement
les PageClient encore vivants a une racine GC. Le benchmark memoire normal est
execute sans dump_graph(); cette preuve provient d'une repetition diagnostic
separee pour ne pas polluer les mesures RSS M0/M1/M2.
"""
from __future__ import annotations

import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
SUMMARY = re.compile(
    r"\[LB:GC_RETENTION\] pid=(\d+) detached=(\d+) active=(\d+) "
    r"pageclients=(\d+) retained=(\d+) paths=(\d+) END"
)
PATH = re.compile(
    r"\[LB:GC_PATH\] pid=(\d+) detached=(\d+) target=([^ ]+) "
    r"root=(.*?) frame=(-?\d+) frame_label=(.*?) depth=(\d+) path=(.*?) END"
)
PAGE_STATE = re.compile(
    r"\[LB:PAGE_STATE\] pid=(\d+) seq=(\d+) roots=(\d+) created=(\d+) "
    r"detached=(\d+) finalized=(\d+) END"
)

CONSERVATIVE = {
    "StackPointer",
    "RegisterPointer",
    "ConservativeVector",
    "ConservativeHashMap",
    "ConservativeHashTable",
    "HeapFunctionCapturedPointer",
}


@dataclass(frozen=True)
class Summary:
    pid: int
    detached: int
    active: int
    pageclients: int
    retained: int
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


def classify_root(root: str) -> str:
    if root in CONSERVATIVE:
        return "conservative"
    if root == "NO_PATH":
        return "unknown"
    if root == "VM" or root.startswith("Root ") or root.startswith("Root"):
        return "strong"
    if root in {"CrossHeapMember", "RootVector", "RootHashMap", "RootHashTable"}:
        return "strong"
    return "unknown"


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

    out: list[str] = []
    if not states:
        return 2, ["GC_RETENTION_INCONCLUSIF PAGE_STATE absent"]

    max_detached_by_pid: dict[int, int] = defaultdict(int)
    for pid, _seq, _roots, _created, detached, _finalized in states:
        max_detached_by_pid[pid] = max(max_detached_by_pid[pid], detached)
    main_pid, max_detached = max(max_detached_by_pid.items(), key=lambda kv: kv[1])
    out.append(f"GC_RETENTION_MAIN pid={main_pid} detached_max={max_detached}")
    if max_detached < 20:
        return 2, out + ["GC_RETENTION_INCONCLUSIF le WebContent ouvreur n'a pas atteint 20 detachements"]

    selected = {s.detached: s for s in summaries if s.pid == main_pid}
    missing = [m for m in (10, 20) if m not in selected]
    if missing:
        return 2, out + [f"GC_RETENTION_INCONCLUSIF jalons_absents={missing}"]

    all_classifications: Counter[str] = Counter()
    for milestone in (10, 20):
        s = selected[milestone]
        expected_retained = max(0, s.pageclients - s.active)
        if expected_retained != s.retained:
            return 2, out + [f"GC_RETENTION_INCONCLUSIF detached={milestone} retained={s.retained} attendu={expected_retained}"]
        if s.active < 1:
            return 2, out + [f"GC_RETENTION_INCONCLUSIF detached={milestone} active={s.active}"]

        pp = [p for p in paths if p.pid == main_pid and p.detached == milestone]
        required_paths = min(s.pageclients, 32)
        if s.paths < required_paths or len(pp) < required_paths:
            return 2, out + [f"GC_RETENTION_INCONCLUSIF detached={milestone} paths={len(pp)}/{required_paths} annonce={s.paths}"]

        classes = Counter(classify_root(p.root) for p in pp)
        all_classifications.update(classes)
        roots = Counter(p.root for p in pp)
        out.append(
            "GC_RETENTION_JALON "
            f"pid={main_pid} detached={milestone} active={s.active} pageclients={s.pageclients} "
            f"retained={s.retained} conservative={classes['conservative']} "
            f"strong={classes['strong']} unknown={classes['unknown']} roots={dict(roots)}"
        )

        shown: set[tuple[str, str, str]] = set()
        for p in pp:
            signature = (p.root, p.frame_label, p.path)
            if signature in shown:
                continue
            shown.add(signature)
            out.append(
                f"GC_RETENTION_PATH detached={milestone} root={p.root} frame={p.frame} "
                f"frame_label={p.frame_label or '<none>'} depth={p.depth} path={p.path}"
            )
            if len(shown) >= 5:
                break

        if s.retained == 0:
            out.append(f"GC_RETENTION_NONE pid={main_pid} detached={milestone}")

    if all_classifications["unknown"]:
        return 2, out + [f"GC_RETENTION_INCONCLUSIF unknown_paths={all_classifications['unknown']}"]

    out.append(
        "GC_RETENTION_PREUVE_OK "
        f"pid={main_pid} conservative={all_classifications['conservative']} "
        f"strong={all_classifications['strong']}"
    )
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
