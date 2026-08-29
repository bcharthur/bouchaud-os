#!/usr/bin/env python3
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[2]
cle = root / "src/kernel/sync/wait_word/cle.rs"
signal = root / "src/kernel/sync/wait_source/signal.rs"
wake = root / "src/kernel/sync/wait_word/reveil.rs"

for p in (cle, signal, wake):
    if not p.exists():
        print("missing:", p)
        raise SystemExit(1)

ct = cle.read_text(encoding="utf-8")
if "let translated = {" not in ct or "let mut mm = process.mm.lock();" not in ct:
    print("wait_word_key mutable lock guard is not explicitly scoped")
    raise SystemExit(2)

st = signal.read_text(encoding="utf-8")
wt = wake.read_text(encoding="utf-8")
if "pub fn signal_one(&self) -> bool" not in st:
    print("WaitSource::signal_one missing")
    raise SystemExit(3)
if "entry.wait.signal_one()" not in wt:
    print("wait_word targeted wake contract missing")
    raise SystemExit(4)

print("V13.1 compile contracts: OK")
