#!/usr/bin/env python3
"""Mesures par session, directement depuis un ZIP blackbox (sans extraction).

Les compteurs HID sont cumulatifs : kbd n'est pas un nombre de frappes.
Les periodes sans entree ne constituent pas un benchmark FPS.
"""
import argparse
import collections
import csv
import io
import json
import re
import zipfile


def analyse(fichiers):
    serial = re.sub(r"\x1b\[[0-9;]*m", "", fichiers.get("serial.log", ""))
    samples = []
    for ligne in fichiers.get("samples.log", "").splitlines():
        valeurs = dict(re.findall(r"\b(\w+)=(\d+)(?=\s|$)", ligne))
        if "ts_ns" in valeurs and "polls" in valeurs:
            samples.append({k: int(v) for k, v in valeurs.items()})
    resultat = {"echantillons": len(samples)}
    if samples:
        premier, dernier = samples[0], samples[-1]
        duree = (dernier["ts_ns"] - premier["ts_ns"]) / 1e9
        resultat["fenetre_echantillonnee_s"] = round(duree, 6)
        resultat["dernier_echantillon_depuis_boot_s"] = dernier["ts_ns"] / 1e9
        delta = dernier["polls"] - premier["polls"]
        resultat["scrutations_par_s"] = round(delta / duree, 2) if duree > 0 and delta >= 0 else None
        resultat["compteurs_finaux"] = {k: dernier.get(k) for k in (
            "polls", "events", "reports", "kbd", "mouse", "errors", "rearms", "kicks",
            "bb_writes", "bb_failures",
        )}
        for compteur in ("wm_tours", "wm_entrees", "wm_trames"):
            if compteur in premier and compteur in dernier and duree > 0:
                resultat[compteur + "_par_s_observe"] = round(
                    (dernier[compteur] - premier[compteur]) / duree, 2)
    evenements = list(csv.DictReader(io.StringIO(fichiers.get("flight.csv", ""))))
    resultat["evenements_de_vol"] = dict(collections.Counter(e.get("kind_name", "inconnu") for e in evenements))
    smp = re.search(r"BOUCHAUD_SMP_BATTEMENT_BILAN battants=(\d+) en_ligne=(\d+)", serial)
    resultat["cpu_avec_timer"] = int(smp[1]) if smp else None
    resultat["cpu_en_ligne"] = int(smp[2]) if smp else None
    resultat["uefi_entree"] = "BOUCHAUD_UEFI_ENTRY_OK" in serial
    resultat["gestionnaire_fenetres_pret"] = "BOUCHAUD_STAGE2_WINDOW_MANAGER_READY" in serial
    resultat["reseau_configure"] = "BOUCHAUD_NET_RECONFIGURE verdict=pret" in serial
    resultat["handshake_navigateur_observe"] = "M11_GUI_HANDSHAKE_OK" in serial
    resultat["journal_fatal_non_vide"] = bool(fichiers.get("fatal.log", "").strip())
    resultat["fps_et_latence_entree_image"] = "non mesures par ces compteurs"
    return resultat


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive")
    args = parser.parse_args()
    sessions = collections.defaultdict(dict)
    with zipfile.ZipFile(args.archive) as archive:
        for entree in archive.infolist():
            chemin = entree.filename.replace("\\", "/")
            if "/" not in chemin or entree.is_dir():
                continue
            session, nom = chemin.rsplit("/", 1)
            if nom in {"serial.log", "samples.log", "flight.csv", "fatal.log"}:
                sessions[session][nom] = archive.read(entree).decode("utf-8-sig", errors="replace")
    if not sessions:
        parser.error("aucune session blackbox trouvee dans le ZIP")
    print(json.dumps({nom: analyse(f) for nom, f in sessions.items()}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
