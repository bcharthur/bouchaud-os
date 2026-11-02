#!/usr/bin/env python3
"""Fail closed on a stale, machine-readable GitHub status snapshot.
Does not modify source, tests, workflows, thresholds or remote branches.
"""
import json
import subprocess
import sys
from pathlib import Path
root = Path(__file__).resolve().parents[2]
p = root / "docs/current/status.json"
data = json.loads(p.read_text(encoding="utf-8"))
expected = data["observed_main_head"]
if not isinstance(expected, str) or len(expected) != 40 or any(c not in "0123456789abcdef" for c in expected):
    sys.exit("STATUS_SNAPSHOT_BAD_SHA")
actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
if actual != expected:
    print("STATUS_SNAPSHOT_STALE actual="+actual+" snapshot="+expected)
    sys.exit(2)
print("STATUS_SNAPSHOT_MATCH "+actual)
