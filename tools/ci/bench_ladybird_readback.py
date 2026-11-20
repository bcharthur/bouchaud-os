#!/usr/bin/env python3
"""Résume les lots de relecture Skia dans un journal série d'un démarrage."""

import argparse
import json
import re
from statistics import median
from pathlib import Path

MARQUEUR = "[LB:SKIA:READBACK] end_us:wall_us:bytes"
PRESENTATION = re.compile(r"\[LB:FRAME\][^\n]*\bpresent_us=(\d+)")
CPU_INVITE = re.compile(r"\[GPU-COMPOSITION\][^\n]*\bcpu_pct=(\d+)")
SEUILS_TRAMES_LONGUES_US = (16_667, 33_333, 50_000, 100_000)
MARQUEUR_GPU = "[LB:SKIA:GPU] stage="
ECHANTILLON = re.compile(r"(\d+):(\d+):(\d+)")


def centiles(durees):
    durees.sort()
    nombre = len(durees)
    return {f"p{p}": durees[(p * nombre + 99) // 100 - 1] if nombre else None for p in (50, 95, 99)}


def resume(journal):
    durees = []
    octets = 0
    echecs = 0
    lots_incomplets = 0
    gpu = {"flush_submit": [], "completion_observed": []}
    for ligne in journal.splitlines():
        if MARQUEUR_GPU in ligne:
            nom, _, valeurs = ligne.split(MARQUEUR_GPU, 1)[1].partition(" wall_us")
            lot = valeurs.split()
            if nom in gpu and len(lot) == 16 and all(re.fullmatch(r"[0-9]+", valeur) for valeur in lot):
                gpu[nom].extend(map(int, lot))
            else:
                lots_incomplets += 1
            continue
        if MARQUEUR not in ligne:
            continue
        lot = ligne.split(MARQUEUR, 1)[1].split()
        if len(lot) != 16 or not all(ECHANTILLON.fullmatch(valeur) for valeur in lot):
            lots_incomplets += 1
            continue
        for valeur in lot:
            _, duree, volume = map(int, valeur.split(":"))
            if volume:
                durees.append(duree)
                octets += volume
            else:
                echecs += 1

    presentations = [int(valeur) for valeur in PRESENTATION.findall(journal)]
    charge_cpu = [int(valeur) for valeur in CPU_INVITE.findall(journal)]
    return {
        "charge_cpu_invite": {"echantillons_pct": charge_cpu,
                              "mediane_pct": median(charge_cpu) if charge_cpu else None},
        "presentation_navigateur": {"echantillons": len(presentations),
                                     "duree_murale_us": centiles(presentations),
                                     "longues_au_dela_us": {
                                         str(seuil): sum(duree > seuil for duree in presentations)
                                         for seuil in SEUILS_TRAMES_LONGUES_US}},
        "lectures_reussies": len(durees),
        "lectures_en_echec": echecs,
        "lots_incomplets": lots_incomplets,
        "octets_relus": octets,
        "octets_relus_par_lecture": octets / len(durees) if durees else None,
        "duree_murale_us": centiles(durees),
        "gpu": {nom: {"echantillons": len(valeurs), "duree_murale_us": centiles(valeurs)}
                for nom, valeurs in gpu.items()},
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("journal", type=Path)
    parser.add_argument("--exige", choices=("readback", "cpu", "vulkan"), default="readback",
                        help="Vérifie les mesures attendues pour le bras demandé.")
    args = parser.parse_args()
    bilan = resume(args.journal.read_text(errors="replace"))
    print(json.dumps(bilan, indent=2))
    if args.exige == "cpu":
        return 0 if bilan["presentation_navigateur"]["echantillons"] and not bilan["lectures_reussies"] else 1
    if args.exige == "vulkan":
        complet = (bilan["presentation_navigateur"]["echantillons"] and bilan["lectures_reussies"]
                   and not bilan["lectures_en_echec"] and not bilan["lots_incomplets"]
                   and all(etape["echantillons"] for etape in bilan["gpu"].values()))
        return 0 if complet else 1
    return 0 if bilan["lectures_reussies"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
