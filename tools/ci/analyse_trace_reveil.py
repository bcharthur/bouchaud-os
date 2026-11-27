#!/usr/bin/env python3
"""Raccorde les échantillons de la sonde aux records RAM de la VM arrêtée."""

import argparse
import csv
import re
import struct
from bisect import bisect_left, bisect_right
from collections import defaultdict
from pathlib import Path

LIGNE = re.compile(r"^ORDO_TRACE phase=(.+?) i=(\d+) (.*)$")
RECORD = struct.Struct("<11Q")
PUBLICATION = struct.Struct("<5Q")
CAPACITE = 32768
N_REVEILS = 300


def records(total_path, records_path, format_record):
    total, = struct.unpack("<Q", total_path.read_bytes())
    if total > CAPACITE:
        raise ValueError(f"trace noyau saturée ({total}/{CAPACITE})")
    records = records_path.read_bytes()
    if len(records) < total * format_record.size:
        raise ValueError("dump de trace noyau tronqué")
    for numero in range(total):
        champs = format_record.unpack_from(records, numero * format_record.size)
        if champs[-1] != numero + 1:
            raise ValueError(f"record noyau incomplet {numero}")
        yield champs[:-1]


def noyau(total_path, records_path):
    index = defaultdict(list)
    for champs in records(total_path, records_path, RECORD):
        index[champs[1] // 1000].append(champs)
    return index


def publications(total_path, records_path):
    index = defaultdict(list)
    for tache, avant, apres, neuve in records(total_path, records_path, PUBLICATION):
        if not 0 < avant <= apres or neuve not in (0, 1):
            raise ValueError("publication de file incohérente")
        if neuve:
            index[tache].append((avant, apres))
    return {tache: sorted(instants) for tache, instants in index.items()}


def raccord_file(trace, index):
    tache, _, _, _, _, _, avant_appel, election, _, _ = trace
    if not tache or not 0 < avant_appel <= election:
        return 0, "", ""
    instants = index.get(tache, [])
    debut = bisect_left(instants, avant_appel, key=lambda mesure: mesure[0])
    fin = bisect_right(instants, election, key=lambda mesure: mesure[0])
    return (1, *instants[debut]) if fin - debut == 1 else (fin - debut, "", "")



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


def rapport(index, phases_trace, index_file=None):
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
                    file = raccord_file(trace, index_file) if index_file is not None else ("", "", "")
                    yield (serie, nom, population, centile, rang, i, valeur, len(candidats), *trace, *file)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("total", type=Path)
    parser.add_argument("records", type=Path)
    parser.add_argument("echantillons", type=Path)
    parser.add_argument("sortie", type=Path)
    parser.add_argument("--file", nargs=2, type=Path, metavar=("TOTAL", "RECORDS"))
    args = parser.parse_args()
    index = noyau(args.total, args.records)
    index_file = publications(*args.file) if args.file else None
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
            "raccords_file", "publication_avant_ns", "publication_apres_ns",
        ))
        csv_out.writerows(rapport(index, series, index_file))


if __name__ == "__main__":
    main()
