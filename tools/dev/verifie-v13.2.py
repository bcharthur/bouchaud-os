#!/usr/bin/env python3
from pathlib import Path

root = Path(__file__).resolve().parents[2]
cle = root / "src/kernel/sync/wait_word/cle.rs"

if not cle.exists():
    print("V13.2 missing:", cle)
    raise SystemExit(1)

text = cle.read_text(encoding="utf-8")

required = [
    "let translated = {",
    "let mut mm = process.mm.lock();",
    "mm.space.translate(uaddr)",
    "translated.or(Some(uaddr))",
]
for token in required:
    if token not in text:
        print("V13.2 contract missing:", token)
        raise SystemExit(2)

for forbidden in [
    "let mm = process.mm.lock();",
    "process.mm.lock().space.translate(uaddr).or(Some(uaddr))",
]:
    if forbidden in text:
        print("V13.2 stale pattern present:", forbidden)
        raise SystemExit(3)

print("V13.2 compile contracts: OK")
