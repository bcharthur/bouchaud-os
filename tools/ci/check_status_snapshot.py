#!/usr/bin/env python3
"""Fail closed on a stale, machine-readable GitHub status snapshot.

The snapshot records the main HEAD that was audited before the documentation
preparation PR. After that docs-only PR is merged, HEAD necessarily changes.
Accept that audited commit itself, or a descendant whose changes since the
snapshot are restricted to docs/current/** plus this checker. Any runtime,
workflow, source or other repository change makes the snapshot stale.
"""
import json
import subprocess
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[2]
p = root / "docs/current/status.json"
data = json.loads(p.read_text(encoding="utf-8"))
expected = data["observed_main_head"]

if (
    not isinstance(expected, str)
    or len(expected) != 40
    or any(c not in "0123456789abcdef" for c in expected)
):
    sys.exit("STATUS_SNAPSHOT_BAD_SHA")

actual = subprocess.check_output(
    ["git", "rev-parse", "HEAD"], cwd=root, text=True
).strip()

if actual == expected:
    print("STATUS_SNAPSHOT_MATCH " + actual)
    raise SystemExit(0)

ancestor = subprocess.run(
    ["git", "merge-base", "--is-ancestor", expected, actual],
    cwd=root,
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
).returncode
if ancestor != 0:
    print("STATUS_SNAPSHOT_STALE actual=" + actual + " snapshot=" + expected)
    raise SystemExit(2)

changed = subprocess.check_output(
    ["git", "diff", "--name-only", expected + ".." + actual],
    cwd=root,
    text=True,
).splitlines()

allowed_exact = {"tools/ci/check_status_snapshot.py"}
unexpected = [
    path
    for path in changed
    if not path.startswith("docs/current/") and path not in allowed_exact
]

if unexpected:
    print("STATUS_SNAPSHOT_STALE actual=" + actual + " snapshot=" + expected)
    for path in unexpected:
        print("STATUS_SNAPSHOT_RUNTIME_CHANGE " + path)
    raise SystemExit(2)

print(
    "STATUS_SNAPSHOT_COMPATIBLE_DOCS_ONLY actual="
    + actual
    + " snapshot="
    + expected
)
