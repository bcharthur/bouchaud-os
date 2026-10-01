#!/usr/bin/env python3
"""L'identite du coeur courant se lit sur GS valide, avec repli APIC explicite.

Ce garde etait la seconde moitie de `verifie-etat-bkl-atomique.py`. La
premiere -- OWNER et profondeur du gros verrou publies en un seul mot
atomique -- est partie avec le verrou (BOUCHAUD_BKL_SUPPRIME_V1). Celle-ci ne
dependait pas de lui : un GS_BASE pointant le mauvais emplacement fait lire
l'etat d'un autre coeur a tous les chemins par-CPU (ordonnanceur, portes de
transition, identite des taches), verrou global ou non.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
USERMODE = RACINE / "src/arch/x86_64/usermode.rs"
SMP = RACINE / "src/arch/x86_64/smp.rs"


def code_seul(source: str) -> str:
    return re.sub(r"//[^\n]*", "", source)


def main() -> int:
    fautes = []
    usermode = code_seul(USERMODE.read_text(encoding="utf-8"))
    smp = code_seul(SMP.read_text(encoding="utf-8"))
    if "pub fn cpu_index_from_gs() -> Option<usize>" not in usermode:
        fautes.append("  usermode.rs  GS_BASE n'est plus valide avant usage comme CpuId")
    if "logical_for_apic(cpu_local::hardware_apic_id())" not in usermode:
        fautes.append("  usermode.rs  l'identite APIC de secours a disparu")
    if "materiel.as_usize() != index" not in usermode:
        fautes.append(
            "  usermode.rs  un GS pointant un autre slot valide n'est plus "
            "refuse par comparaison avec l'APIC materiel"
        )
    if "if let Some(via_gs) = usermode::cpu_index_from_gs()" not in smp:
        fautes.append("  smp.rs  un GS absent peut de nouveau etre confondu avec CPU0")

    if fautes:
        print("identite CPU : regle violee")
        print("\n".join(fautes))
        return 1

    print("ok  identite CPU : GS valide, repli APIC explicite")
    return 0


if __name__ == "__main__":
    sys.exit(main())
