#!/usr/bin/env python3
"""Identite commune TSC_AUX etiquetee, puis GS valide et repli APIC explicite.

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


def verifie(usermode: str, smp: str) -> list[str]:
    fautes = []
    usermode = code_seul(usermode)
    smp = code_seul(smp)
    if "pub fn cpu_index_from_gs() -> Option<usize>" not in usermode:
        fautes.append("  usermode.rs  GS_BASE n'est plus valide avant usage comme CpuId")
    if "logical_for_apic(cpu_local::hardware_apic_id())" not in usermode:
        fautes.append("  usermode.rs  l'identite APIC de secours a disparu")
    if "materiel.as_usize() != index" not in usermode:
        fautes.append(
            "  usermode.rs  un GS pointant un autre slot valide n'est plus "
            "refuse par comparaison avec l'APIC materiel"
        )
    if not re.search(r"pub fn cpu_index\(\) -> usize\s*\{\s*usermode::cpu_index\(\)\s*\}", smp):
        fautes.append("  smp.rs  l'identite du timer contourne la primitive commune verifiee")
    if "if let Some(index) = index_par_tsc_aux()" not in usermode:
        fautes.append("  usermode.rs  le chemin rapide TSC_AUX a disparu")
    if "aux & 0xFFFF_0000 != TSC_AUX_ETIQUETTE" not in usermode:
        fautes.append("  usermode.rs  un TSC_AUX non initialise peut etre accepte")
    if "cpu_index_from_gs()\n        .or_else(" not in usermode:
        fautes.append("  usermode.rs  le repli GS/APIC a disparu")
    return fautes


def main() -> int:
    usermode = USERMODE.read_text(encoding="utf-8")
    smp = SMP.read_text(encoding="utf-8")
    fautes = verifie(usermode, smp)
    mutations = [
        (usermode, smp.replace("    usermode::cpu_index()", "    0")),
        (usermode, smp.replace("    usermode::cpu_index()", "    usermode::cpu_index_from_gs().unwrap_or(0)")),
        (usermode.replace("materiel.as_usize() != index", "false"), smp),
        (usermode.replace("if let Some(index) = index_par_tsc_aux()", "if let Some(index) = None"), smp),
        (usermode.replace("aux & 0xFFFF_0000 != TSC_AUX_ETIQUETTE", "false"), smp),
        (usermode.replace("cpu_index_from_gs()\n        .or_else(", "None\n        .or_else("), smp),
    ]
    for n, (u, s) in enumerate(mutations, 1):
        if (u, s) == (usermode, smp) or not verifie(u, s):
            fautes.append(f"  mutation negative {n} non detectee")

    if fautes:
        print("identite CPU : regle violee")
        print("\n".join(fautes))
        return 1

    print("ok  identite CPU commune : TSC_AUX etiquete, GS valide, repli APIC ; 6 mutations refusees")
    return 0


if __name__ == "__main__":
    sys.exit(main())
