#!/usr/bin/env python3
"""Refuse le retour d'un OWNER et d'une profondeur publies separement."""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
ETAT = RACINE / "src/kernel/sync/bkl/etat.rs"
ACQUISITION = RACINE / "src/kernel/sync/bkl/acquisition"
ORDONNANCEUR = RACINE / "src/kernel/sync/bkl/ordonnanceur"
USERMODE = RACINE / "src/arch/x86_64/usermode.rs"
SMP = RACINE / "src/arch/x86_64/smp.rs"


def code_seul(source: str) -> str:
    return re.sub(r"//[^\n]*", "", source)


def main() -> int:
    fautes = []
    etat = code_seul(ETAT.read_text(encoding="utf-8"))
    chemins = "\n".join(
        code_seul(path.read_text(encoding="utf-8"))
        for dossier in (ACQUISITION, ORDONNANCEUR)
        for path in sorted(dossier.glob("*.rs"))
    )

    if "static ETAT: AtomicU64" not in etat:
        fautes.append("  etat.rs  le mot atomique OWNER+DEPTH a disparu")
    if re.search(r"static\s+OWNER\s*:\s*Atomic", etat):
        fautes.append("  etat.rs  OWNER est de nouveau publie separement")
    if re.search(r"static\s+DEPTH\s*:\s*\[", etat):
        fautes.append("  etat.rs  DEPTH est de nouveau publie separement")
    for primitive in (
        "encode_etat(",
        "decode_etat(",
        "essaie_acquerir_etat(",
        "remplace_profondeur_possedee(",
        "augmente_profondeur(",
    ):
        if primitive not in etat:
            fautes.append(f"  etat.rs  primitive atomique absente : `{primitive}`")

    if re.search(r"\bOWNER\.(?:store|compare_exchange)", chemins):
        fautes.append("  chemins BKL  transition directe de OWNER detectee")
    if re.search(r"\bDEPTH\[[^]]+\]\.store", chemins):
        fautes.append("  chemins BKL  transition directe de DEPTH detectee")
    for appel in ("essaie_acquerir_etat(", "remplace_profondeur_possedee("):
        if appel not in chemins:
            fautes.append(f"  chemins BKL  la primitive `{appel}` n'est plus utilisee")

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
        print("etat BKL atomique : regle violee")
        print("\n".join(fautes))
        return 1

    print("ok  BKL : OWNER+DEPTH indivisibles ; GS valide, repli APIC explicite")
    return 0


if __name__ == "__main__":
    sys.exit(main())
