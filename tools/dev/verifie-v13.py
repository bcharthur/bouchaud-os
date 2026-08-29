#!/usr/bin/env python3
from pathlib import Path
import sys
root=Path(__file__).resolve().parents[2]
required=[
 'src/kernel/sync/wait_word.rs','src/kernel/sync/wait_word/attente.rs',
 'src/fs/persistance/snapshot.rs','src/fs/persistance/transaction.rs','src/fs/persistance/io.rs',
 'src/kernel/memory/readahead.rs','src/gui/desktop_bkl/politique.rs',
 'tools/perf/profile-v13.py','tools/perf/run-ladybird-fast.ps1']
missing=[p for p in required if not (root/p).exists()]
if missing:
 print('V13 missing:',*missing,sep='\n - '); sys.exit(1)
for p in required:
 data=(root/p).read_bytes()
 if data.startswith(b'\xef\xbb\xbf') or b'\r\n' in data:
  print('encoding/line ending invalid:',p); sys.exit(2)

# V13.1 compile-contract checks.
cle = (root/'src/kernel/sync/wait_word/cle.rs').read_text(encoding='utf-8')
if 'let mut mm = process.mm.lock();' not in cle or 'let translated = {' not in cle:
 print('V13.1 wait_word key guard scope missing'); sys.exit(3)

signal = (root/'src/kernel/sync/wait_source/signal.rs').read_text(encoding='utf-8')
wake = (root/'src/kernel/sync/wait_word/reveil.rs').read_text(encoding='utf-8')
if 'pub fn signal_one(&self) -> bool' not in signal:
 print('V13 contract missing: WaitSource::signal_one'); sys.exit(4)
if 'entry.wait.signal_one()' not in wake:
 print('V13 contract missing: wait_word targeted wake'); sys.exit(5)

print('V13.1 structure + compile contracts: OK')
