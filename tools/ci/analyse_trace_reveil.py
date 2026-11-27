#!/usr/bin/env python3
"""Raccorde les échantillons de la sonde aux records RAM de la VM arrêtée."""

import argparse
import csv
import re
import struct
from collections import defaultdict
from pathlib import Path

LIGNE = re.compile(r"^ORDO_TRACE phase=(.+?) i=(\d+) (.*)$")
RECORD = struct.Struct("<11Q")
CAPACITE = 32768
N_REVEILS = 300


def noyau(total_path, records_path):
    total, = struct.unpack("<Q", total_path.read_bytes())
    if total > CAPACITE:
        raise ValueError(f"trace noyau saturée ({total}/{CAPACITE})")
    records = records_path.read_bytes()
    if len(records) < total * RECORD.size:
        raise ValueError("dump de trace noyau tronqué")
    index = defaultdict(list)
    for numero in range(total):
        champs = RECORD.unpack_from(records, numero * RECORD.size)
        if champs[-1] != numero + 1:
            raise ValueError(f"record noyau incomplet {numero}")
        index[champs[1] // 1000].append(champs[:-1])
    return index


def phases(path):
    phase, valeurs = None, []
    for ligne in path.read_text().splitlines():
        match = LIGNE.fullmatch(ligne)
        if not match:
            raise ValueError(f"ligne inattendue {ligne!r}")
        nom, indice, texte = match.groups()
        if indice == "0" and not valeurs:
            phase = nom
        if phase != nom or int(indice) != len(valeurs):
            raise ValueError(f"phase discontinue {nom!r} échantillon {indice}")
        valeurs.append({k: int(v) for k, v in (x.split("=", 1) for x in texte.split())})
        if len(valeurs) == N_REVEILS:
            yield phase, valeurs
            phase, valeurs = None, []
    if valeurs:
        raise ValueError(f"dernière phase incomplète {phase!r}")


def rapport(index, phases_trace):
    for serie, (nom, valeurs) in enumerate(phases_trace, 1):
        for population, indices in (
            ("tout", list(range(len(valeurs)))),
            ("propre", [i for i, v in enumerate(valeurs) if not v["ecart"]]),
        ):
            if not indices:
                continue
            ordonnes = sorted(valeurs[i]["retard_us"] for i in indices)
            n = len(ordonnes)
            for centile, rang in (("p95", n * 95 // 100), ("p99", n * 99 // 100), ("max", n - 1)):
                valeur = ordonnes[rang]
                for i in indices:
                    echantillon = valeurs[i]
                    if echantillon["retard_us"] != valeur:
                        continue
                    candidats = index.get(echantillon["entree_noyau_us"], [])
                    trace = candidats[0] if len(candidats) == 1 else ("",) * 10
                    yield (serie, nom, population, centile, rang, i, valeur, len(candidats), *trace)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("total", type=Path)
    parser.add_argument("records", type=Path)
    parser.add_argument("echantillons", type=Path)
    parser.add_argument("sortie", type=Path)
    args = parser.parse_args()
    index = noyau(args.total, args.records)
    series = list(phases(args.echantillons))
    if not series or len(series) % 3:
        raise ValueError("le relevé doit contenir les trois phases de chaque tentative")
    with args.sortie.open("w", newline="") as fichier:
        csv_out = csv.writer(fichier)
        csv_out.writerow((
            "serie", "phase", "population", "centile", "rang_zero", "echantillon",
            "retard_us", "raccords_noyau", "tache", "entree_ns", "echeance_ns",
            "cpu_scan_plus_un", "irq_tsc", "debut_scan_ns", "avant_file_ns",
            "election_ns", "reprise_ns", "cpu_reprise",
        ))
        csv_out.writerows(rapport(index, series))


if __name__ == "__main__":
    main()
