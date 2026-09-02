#!/usr/bin/env python3
"""Fail-closed scanner for Bouchaud OS serial journals.

The serial console is multi-CPU and can interleave fragments.  Fatal matching
therefore uses strong, explicit markers only.  Diagnostic counters such as
`[LOCKDEP] ... violations=0` and BKL traces containing `uaf=` are NOT failures.
"""

from __future__ import annotations

import argparse
import json
import re
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import Iterable

FATAL_PATTERNS = (
    ("kernel_panic", re.compile(r"\*\*\*\s*KERNEL PANIC\s*\*\*\*|panicked at", re.I)),
    ("double_fault", re.compile(r"\bDOUBLE FAULT\b", re.I)),
    ("triple_fault", re.compile(r"\bTRIPLE FAULT\b", re.I)),
    ("bkl_violation", re.compile(
        r"\bBKL(?:-FR)?\b.*\bVIOLATION\b|smp_lock:.*(?:panic|violation)",
        re.I,
    )),
    # Explicit LOCKDEP fatal markers or a non-zero violations counter.
    # Do not match the benign telemetry line:
    #   [LOCKDEP] acquisitions=270 violations=0 max_depth=1
    ("lockdep", re.compile(
        r"\bLOCKDEP\b.*(?:\bFATAL\b|\bPANIC\b|\bVIOLATION\b(?!S=0\b)|"
        r"\bviolations=[1-9][0-9]*\b)|lock order inversion",
        re.I,
    )),
    # `uaf=` occurs in BKL detached diagnostics and is not by itself a UAF.
    # Keep only unambiguous failure spellings.
    ("use_after_free", re.compile(
        r"\buse[- ]after[- ]free\b|\[\s*UAF\s*\]|\bUAF\s*:\s*(?:FATAL|PANIC|DETECTED|VIOLATION)\b",
        re.I,
    )),
    ("assertion", re.compile(r"\bassertion .* failed\b", re.I)),
    # Une tache `Ready`, sur aucun coeur, absente de la file de son propre
    # `runq_cpu`, n'est dans AUCUNE file : aucun `pick_next` ne la trouvera et
    # personne ne la republiera. Le noyau se fige alors sans rien imprimer --
    # c'est exactement la regression mm-ng6 SMP4, restee cinq minutes muette.
    # Le marqueur n'est emis que lorsque l'invariant est deja rompu : sa seule
    # presence est la faute.
    ("scheduler_orphan", re.compile(r"\[SCHED-ORPHELINE\]")),
)

@dataclass(frozen=True)
class Finding:
    kind: str
    line: int
    text: str

@dataclass(frozen=True)
class ScanResult:
    path: str
    bytes: int
    lines: int
    findings: tuple[Finding, ...]
    required_markers_missing: tuple[str, ...]

    @property
    def ok(self) -> bool:
        return not self.findings and not self.required_markers_missing and self.bytes > 0

def scan_text(
    text: str,
    *,
    required_markers: Iterable[str] = (),
) -> tuple[tuple[Finding, ...], tuple[str, ...]]:
    findings: list[Finding] = []
    lines = text.splitlines()
    for number, line in enumerate(lines, 1):
        for kind, pattern in FATAL_PATTERNS:
            if pattern.search(line):
                findings.append(Finding(kind, number, line[:500]))
                break
    missing = tuple(marker for marker in required_markers if marker not in text)
    return tuple(findings), missing

def scan_file(path: Path, *, required_markers: Iterable[str] = ()) -> ScanResult:
    raw = path.read_bytes() if path.exists() else b""
    text = raw.decode("utf-8", errors="replace")
    findings, missing = scan_text(text, required_markers=required_markers)
    return ScanResult(
        path=str(path),
        bytes=len(raw),
        lines=len(text.splitlines()),
        findings=findings,
        required_markers_missing=missing,
    )

def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("journals", nargs="+", type=Path)
    parser.add_argument("--require", action="append", default=[], help="marker required in every journal")
    parser.add_argument("--json", dest="json_path", type=Path)
    args = parser.parse_args()

    results = [scan_file(p, required_markers=args.require) for p in args.journals]
    payload = {
        "schema": 2,
        "ok": all(r.ok for r in results),
        "journals": [
            {
                **{k: v for k, v in asdict(r).items() if k != "findings"},
                "ok": r.ok,
                "findings": [asdict(f) for f in r.findings],
            }
            for r in results
        ],
    }

    for result in results:
        print(f"{'OK' if result.ok else 'ECHEC'} {result.path}: {result.bytes} octets, {result.lines} lignes")
        for finding in result.findings:
            print(f"  {finding.kind}:{finding.line}: {finding.text}")
        for marker in result.required_markers_missing:
            print(f"  marqueur absent: {marker}")

    if args.json_path:
        args.json_path.parent.mkdir(parents=True, exist_ok=True)
        args.json_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    return 0 if payload["ok"] else 1

if __name__ == "__main__":
    raise SystemExit(main())
