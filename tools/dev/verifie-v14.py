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
 "docs/historique/notes/V14-SOURCE.patch",
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
# V14 exigeait la constante `5 * TICKS_PER_SECOND`. Le CONTRAT qu'elle
# protegeait -- ne pas noyer une entree-sortie serie emulee sous TCG -- ne
# change pas : il ne s'applique plus qu'au cas ou un COM1 existe REELLEMENT.
# Sans port, les lignes tombent dans le tambour RAM et ne coutent rien ; cinq
# secondes y etaient une seule chance de voir, et le releve physique du
# 16 septembre 20:08, arrete a 7,87 s, n'en a eu qu'une.
assert '5 * crate::kernel::timer::TICKS_PER_SECOND' in stall, \
    "la cadence lente a disparu : une entree-sortie serie emulee serait noyee"
assert 'presence_com1' in stall, \
    "la cadence ne depend plus du port : elle est lente partout, ou couteuse partout"
metrics=(root/'src/kernel/process/thread/metriques.rs').read_text(encoding='utf-8')
assert '[MM-CLUSTER]' in metrics and 'periode_rapport = 10 *' in metrics
# Le correctif de reference a rejoint docs/historique/notes/ lors du
# rangement de la racine. Le CONTRAT ne change pas : les memes jetons
# doivent s'y trouver, seule l'adresse a bouge.
patch=(root/'docs/historique/notes/V14-SOURCE.patch').read_text(encoding='utf-8')
for token in ('nr::WRITE','nr::WRITEV','nr::MUNMAP','nr::MADVISE','MAX_RECLAIMABLE_PAGES','READAHEAD_MID'):
 if token not in patch: raise SystemExit(f"V14 source patch missing token: {token}")
py_compile.compile(str(root/'tools/perf/analyse-v14.py'), doraise=True)
print('V14 structure + performance contracts: OK')
print('Then: git apply --check docs/historique/notes/V14-SOURCE.patch')
