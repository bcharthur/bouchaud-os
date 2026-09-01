#!/usr/bin/env python3
"""Regression guard for the /persist RAMFS traversal.

Ignores comments before looking for executable `fs()` calls. This avoids false
positives caused by explanatory comments such as ``aucun appel a fs()``.
"""

from pathlib import Path
import re
import sys

path = Path("src/fs/persistance/collecte.rs")
text = path.read_text(encoding="utf-8")


def strip_rust_comments(source: str) -> str:
    """Remove // and /* ... */ comments while preserving strings."""
    out = []
    i = 0
    n = len(source)
    in_string = False
    escaped = False

    while i < n:
        ch = source[i]
        nxt = source[i + 1] if i + 1 < n else ""

        if in_string:
            out.append(ch)
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                in_string = False
            i += 1
            continue

        if ch == '"':
            in_string = True
            out.append(ch)
            i += 1
            continue

        if ch == "/" and nxt == "/":
            i += 2
            while i < n and source[i] != "\n":
                i += 1
            if i < n:
                out.append("\n")
                i += 1
            continue

        if ch == "/" and nxt == "*":
            i += 2
            depth = 1
            while i < n and depth:
                if i + 1 < n and source[i] == "/" and source[i + 1] == "*":
                    depth += 1
                    i += 2
                elif i + 1 < n and source[i] == "*" and source[i + 1] == "/":
                    depth -= 1
                    i += 2
                else:
                    if source[i] == "\n":
                        out.append("\n")
                    i += 1
            continue

        out.append(ch)
        i += 1

    return "".join(out)


code = strip_rust_comments(text)
errors = []

if "collecte_sous_garde(&systeme" not in code:
    errors.append("rassemble() ne transmet plus le FileSystem protege")

# Locate the recursive walker and extract its body with balanced braces.
sig = re.search(
    r"fn\s+collecte_sous_garde\s*\((.*?)\)\s*\{",
    code,
    flags=re.S,
)
if not sig:
    errors.append("collecte_sous_garde() absente")
else:
    signature = sig.group(1)
    if "FileSystem" not in signature:
        errors.append("collecte_sous_garde() ne recoit pas le FileSystem")

    start = sig.end() - 1
    depth = 0
    end = None
    for pos in range(start, len(code)):
        if code[pos] == "{":
            depth += 1
        elif code[pos] == "}":
            depth -= 1
            if depth == 0:
                end = pos
                break

    if end is None:
        errors.append("corps de collecte_sous_garde() mal forme")
    else:
        body = code[start + 1:end]
        if re.search(r"\bfs\s*\(\s*\)", body):
            errors.append("collecte_sous_garde() reacquiert RAMFS")

# Executable acquisitions expected in this fragment:
# - rassemble(): one read-side acquisition
# - depose(): one write-side acquisition
calls = len(re.findall(r"\bfs\s*\(\s*\)", code))
if calls != 2:
    errors.append(
        f"nombre inattendu d'acquisitions RAMFS executables dans collecte.rs: "
        f"{calls} (attendu 2)"
    )

if errors:
    for error in errors:
        print("ECHEC:", error, file=sys.stderr)
    raise SystemExit(1)

print("RAMFS_COLLECTE_LOCK_OK")
