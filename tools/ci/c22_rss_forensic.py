#!/usr/bin/env python3
"""Forensic C22: publier categories RSS aux reperes M0/M1/M2, sans verdict adouci."""
import argparse
import re
from pathlib import Path

ANSI = re.compile(r"\x1b\[[0-9;]*m")
REP = re.compile(r"HOST_MEMOIRE_REPERE m=([012]) n=([0-9]+)")
RSS = re.compile(r"\[C22:RSS\] t=(\d+) pid=(\d+) rss_kio=(\d+) anon_kio=(\d+) fichier_kio=(\d+) partage_kio=(\d+) device_kio=(\d+) sans_type_kio=(\d+) vma=(\d+)")
FIELDS = ("rss", "anon", "fichier", "partage", "device", "sans_type", "vma")

def analyser(texte):
    lignes = ANSI.sub("", texte).splitlines()
    reperes = {}
    categories = []
    perf = []
    proc = re.compile(r"\[PERF-PROC\] t=(\d+) pid=(\d+) image=(\S+) rss_kio=(\d+)")
    for idx, ligne in enumerate(lignes):
        m = REP.search(ligne)
        if m:
            reperes.setdefault(int(m.group(1)), idx)
        m = RSS.search(ligne)
        if m and int(m.group(2)) == 20:
            categories.append((idx, int(m.group(1)), tuple(map(int, m.groups()[2:]))))
        m = proc.search(ligne)
        if m and int(m.group(2)) == 20 and m.group(3).split("/")[-1] == "WebContent":
            perf.append((idx, int(m.group(1)), int(m.group(4))))
    if set(reperes) != {0, 1, 2} or not reperes[0] < reperes[1] < reperes[2]:
        raise ValueError(f"reperes absents ou desordonnes: {reperes}")
    if not categories:
        raise ValueError("aucune categorie C22:RSS dans ce journal (historique RSS seulement)")
    indicateurs = {}
    for phase in range(3):
        # Le banc contractuel choisit le dernier releve strictement AVANT le repere.
        precedent = [v for i, t, v in categories if i < reperes[phase]]
        controle = [(t, rss) for i, t, rss in perf if i < reperes[phase]]
        if not precedent or not controle:
            raise ValueError(f"M{phase}: releve frais avant repere absent")
        valeurs = precedent[-1]
        if valeurs[0] != controle[-1][1] or valeurs[0] != sum(valeurs[1:6]):
            raise ValueError(f"M{phase}: C22:RSS et PERF-PROC incoherents")
        indicateurs[phase] = valeurs
    return indicateurs

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("journal")
    ap.add_argument("--label", default="run")
    args = ap.parse_args()
    try:
        samples = analyser(Path(args.journal).read_text(errors="replace"))
    except ValueError as exc:
        raise SystemExit(f"C22_FORENSIC_INCONCLUSIF {exc}")
    for p, row in samples.items():
        print("C22_RSS_REPERE", f"cas={args.label}", f"phase=M{p}",
              *[f"{key}={value}" for key, value in zip(FIELDS, row)])
    print("C22_RSS_DELTA", f"cas={args.label}", "M2-M1",
          *[f"{key}={samples[2][i]-samples[1][i]}" for i, key in enumerate(FIELDS)])
    if samples[2][0] != sum(samples[2][1:6]):
        raise SystemExit("C22_FORENSIC_INCONCLUSIF somme des categories RSS incoherente")
    print("C22_FORENSIC_OK (diagnostic, ne remplace pas le verdict contractuel)")

if __name__ == "__main__":
    main()
