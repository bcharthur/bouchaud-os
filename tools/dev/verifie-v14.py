#!/usr/bin/env python3
from pathlib import Path
import re, subprocess, sys, py_compile
root=Path(__file__).resolve().parents[2]
required=[
 "src/kernel/memory/readahead.rs",
 "src/kernel/memory/readahead/politique.rs",
 "src/kernel/process/thread/faute_cluster.rs",
 "src/kernel/process/thread/faute_memoire.rs",
 "src/kernel/process/thread/metriques.rs",
 "src/kernel/process/thread/diagnostic_stall.rs",
 "tools/perf/run-ladybird-v14.ps1",
 "tools/perf/analyse-v14.py",
 "V14-SOURCE.patch",
]
for rel in required:
 p=root/rel
 if not p.exists(): raise SystemExit(f"V14 missing: {rel}")
for p in (root/'src/kernel/memory/readahead').glob('*.rs'):
 if '//!' in p.read_text(encoding='utf-8'):
  raise SystemExit(f"nested include has //!: {p}")
for p in [root/'src/kernel/process/thread/faute_cluster.rs']:
 if '//!' in p.read_text(encoding='utf-8'):
  raise SystemExit(f"nested include has //!: {p}")
policy=(root/'src/kernel/memory/readahead/politique.rs').read_text(encoding='utf-8')
assert 'RA_START_AFTER: u64 = 2' in policy and 'RA_MAX_PAGES: u64 = 16' in policy
fault=(root/'src/kernel/process/thread/faute_memoire.rs').read_text(encoding='utf-8')
assert 'include!("faute_cluster.rs")' in fault and 'fault_cluster_after_clean' in fault
stall=(root/'src/kernel/process/thread/diagnostic_stall.rs').read_text(encoding='utf-8')
assert 'snapshot_period = 5 * crate::kernel::timer::TICKS_PER_SECOND' in stall
metrics=(root/'src/kernel/process/thread/metriques.rs').read_text(encoding='utf-8')
assert '[MM-CLUSTER]' in metrics and 'periode_rapport = 10 *' in metrics
patch=(root/'V14-SOURCE.patch').read_text(encoding='utf-8')
for token in ('nr::WRITE','nr::WRITEV','nr::MUNMAP','nr::MADVISE','MAX_RECLAIMABLE_PAGES','READAHEAD_MID'):
 if token not in patch: raise SystemExit(f"V14 source patch missing token: {token}")
py_compile.compile(str(root/'tools/perf/analyse-v14.py'), doraise=True)
print('V14 structure + performance contracts: OK')
print('Then: git apply --check .\\V14-SOURCE.patch')
