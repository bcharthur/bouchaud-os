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
# V14 exigeait la constante `5 * TICKS_PER_SECOND`, puis une cadence qui
# dependait de la presence d'un COM1. Le CONTRAT qu'elles protegeaient -- ne
# pas noyer le journal sous un diagnostic periodique -- ne change pas ; ce qui
# change, c'est qu'il ne suffisait plus.
#
# Le releve physique du 17 septembre, 769 s d'activite, montre ou menait la
# cadence rapide « qui ne coute rien » sur une machine sans port serie :
#
#     capacite ~1 MiB, produit ~10,4 MiB
#     tambour_reserves=12736 ecrases=4544 perdus=4209
#     premier enregistrement survivant : seq=4546, t~287 s
#
# Tout le demarrage efface par le diagnostic lui-meme. Le contrat devient donc
# plus fort, et il ne depend plus du port : en regime NORMAL, un resume d'une
# ligne, et rien de plus ; l'etat complet ne sort que sur ANOMALIE.
import re as _re
periode = _re.search(
    r'periode_resume\s*=\s*(\d+)\s*\*\s*crate::kernel::timer::TICKS_PER_SECOND',
    stall,
)
assert periode is not None, \
    "la cadence du regime normal n'est plus une constante lisible"
assert int(periode.group(1)) >= 5, \
    "le regime normal imprime plus souvent que toutes les cinq secondes : " \
    "le diagnostic effacerait de nouveau ce qu'il doit expliquer"
assert 'fn resume_ordonnancement()' in stall, \
    "le resume compact a disparu : il ne resterait que le silence ou le deluge"
# L'ETAT COMPLET NE DOIT PAS ETRE PERIODIQUE. C'est lui qui produisait 2150
# lignes `[SCHED-TACHE]` et 1232 `[SCHED-FILE]` par mebioctet de trace.
appel = stall.find('signale_etat_ordonnancement();')
garde = stall.rfind('if complet {', 0, appel)
assert garde != -1 and appel - garde < 300, \
    "l'etat complet de l'ordonnancement est redevenu periodique"
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
