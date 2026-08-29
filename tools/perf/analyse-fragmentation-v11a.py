#!/usr/bin/env python3
"""Localise le dernier freeze BKL V11A à partir des sites 600..699."""

from __future__ import annotations
import re
import sys
from pathlib import Path

SITES = {
    600: "try_enter: début",
    601: "try_enter: lecture OWNER",
    602: "try_enter: avant helper nouvel entrant",
    610: "helper: contrôle RESUME avant CAS",
    611: "helper: contrôle HANDOFF avant CAS",
    612: "helper: juste avant CAS OWNER FREE->mine",
    613: "helper: CAS réussi, OWNER acquis",
    614: "helper: contrôle RESUME après CAS",
    615: "helper: contrôle HANDOFF après CAS",
    616: "helper: avant claim handoff",
    617: "helper: claim handoff terminé",
    618: "helper: refus/rollback",
    620: "try_enter: helper revenu avec succès",
    621: "try_enter: profondeur lue",
    622: "try_enter: DEPTH=1 publié",
    623: "try_enter: avant probe_note_acquire",
    624: "try_enter: probe_note_acquire terminé",
    625: "try_enter: avant flight-recorder",
    626: "try_enter: flight-recorder terminé",
    627: "try_enter: juste avant retour KernelGuard",
    630: "try_enter: chemin réentrant, avant DEPTH++",
    631: "try_enter: chemin réentrant, DEPTH++ terminé",
    639: "try_enter: retour réentrant",
    640: "try_enter: OWNER déjà occupé",
    641: "try_enter: helper a refusé",
    650: "try_enter_depuis_zero: lecture OWNER",
    651: "try_enter_depuis_zero: OWNER non libre",
    652: "try_enter_depuis_zero: avant helper",
    653: "try_enter_depuis_zero: helper a refusé",
    660: "try_enter_depuis_zero: helper réussi",
    661: "try_enter_depuis_zero: DEPTH=1",
    662: "try_enter_depuis_zero: probe terminé",
    663: "try_enter_depuis_zero: retour",
}

def main() -> int:
    if len(sys.argv) != 2:
        print("usage: analyse-fragmentation-v11a.py <log>")
        return 2

    lines = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace").splitlines()
    stall = [line for line in lines if "[SMP-STALL]" in line or "[SMP-SNAPSHOT]" in line]
    if not stall:
        print("Aucune ligne SMP-SNAPSHOT/SMP-STALL.")
        return 3

    mapped = []
    for line in stall:
        m = re.search(r"site=\[([0-9]+):", line)
        if not m:
            continue
        site = int(m.group(1))
        if 600 <= site <= 699:
            mapped.append((site, line))

    print("=== Bouchaud V11A / localisation acquisition BKL ===")
    if not mapped:
        print("Aucun site 600..699 observé.")
        print("Si le freeze reste sur site=60, vérifier que V11A est bien dans l'image.")
        return 0

    site, line = mapped[-1]
    print(f"dernier site V11A CPU0 : {site}")
    print(f"phase                  : {SITES.get(site, 'site V11A inconnu')}")
    print("ligne:")
    print(line)

    if site == 612:
        print("\nVerdict: blocage autour du CAS OWNER.")
    elif 613 <= site <= 617:
        print("\nVerdict: OWNER est acquis; blocage dans la politique post-CAS/handoff.")
    elif site in (623, 624):
        print("\nVerdict: regarder probe_note_acquire / comptabilité / provenance.")
    elif site in (625, 626):
        print("\nVerdict: regarder le flight recorder BKL.")
    elif site == 627:
        print("\nVerdict: try_enter a fini son travail; regarder l'épilogue/retour vers le timer.")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
