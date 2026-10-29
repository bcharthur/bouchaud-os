#!/usr/bin/env python3
"""C22, VALIDATION SEULEMENT : la chronologie d'une repetition memoire, de M0
a la fin, pour separer un rouge corrige des verts : reperes, collectes
[LB:C22], pages creees/finalisees [LB:PAGES], balayages GC, RSS de WebContent
(un releve sur trois). Ne juge rien.

    tools/ci/c22_chronologie.py JOURNAL_SERIE [MAX_LIGNES]
"""
import re
import sys
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
GARDE = re.compile(r"HOST_MEMOIRE_REPERE|HOST_MEMOIRE_ONGLET|\[LB:C22\]|\[LB:PAGES\]|\[LB:GC|GC::|[Gg]arbage|sweep|LibGC|\[PERF-PROC\] .*image=\S*WebContent")
lignes = ANSI.sub("", Path(sys.argv[1]).read_text(errors="replace")).replace("\r", "").splitlines()
plafond = int(sys.argv[2]) if len(sys.argv) > 2 else 90
debut = next((i for i, l in enumerate(lignes) if "HOST_MEMOIRE_REPERE m=0" in l), 0)
sortie, perf, onglets = [], 0, 0
for l in lignes[debut:]:
    if not GARDE.search(l):
        continue
    if "HOST_MEMOIRE_ONGLET" in l:
        onglets += 1
        if onglets % 5:
            continue
    if "[PERF-PROC]" in l:
        perf += 1
        if perf % 3:
            continue
        l = re.sub(r" fautes=.*", "", l)
    sortie.append("  " + l[:220])
if len(sortie) > plafond:
    sortie = sortie[: plafond // 3] + [f"  ... {len(sortie) - plafond} lignes ..."] + sortie[-(plafond - plafond // 3):]
print("\n".join(sortie))
