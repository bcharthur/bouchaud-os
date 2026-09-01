#!/usr/bin/env python3
"""Compare two release artifacts byte-for-byte and explain divergence."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()

def first_difference(a: Path, b: Path) -> int | None:
    offset = 0
    with a.open("rb") as fa, b.open("rb") as fb:
        while True:
            ba = fa.read(1 << 20)
            bb = fb.read(1 << 20)
            if ba == bb:
                if not ba:
                    return None
                offset += len(ba)
                continue
            common = min(len(ba), len(bb))
            for i in range(common):
                if ba[i] != bb[i]:
                    return offset + i
            return offset + common

def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("first", type=Path)
    p.add_argument("second", type=Path)
    p.add_argument("--json", dest="json_path", type=Path)
    args = p.parse_args()

    for path in (args.first, args.second):
        if not path.is_file():
            raise SystemExit(f"artefact absent: {path}")

    s1, s2 = sha256(args.first), sha256(args.second)
    diff = first_difference(args.first, args.second)
    payload = {
        "schema": 1,
        "first": {"path": str(args.first), "bytes": args.first.stat().st_size, "sha256": s1},
        "second": {"path": str(args.second), "bytes": args.second.stat().st_size, "sha256": s2},
        "identical": diff is None,
        "first_difference_offset": diff,
    }
    if args.json_path:
        args.json_path.parent.mkdir(parents=True, exist_ok=True)
        args.json_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    if diff is None:
        print(f"REPRODUCIBLE {s1}")
        return 0
    print(f"NON_REPRODUCTIBLE offset={diff} sha1={s1} sha2={s2}")
    return 1

if __name__ == "__main__":
    raise SystemExit(main())
