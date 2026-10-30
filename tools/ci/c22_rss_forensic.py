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
    indicateurs = {}
    fenetres = {0: [], 1: [], 2: []}
    phase = -1
    for ligne in lignes:
        m = REP.search(ligne)
        if m:
            phase = int(m.group(1))
        m = RSS.search(ligne)
        if m and int(m.group(2)) == 20 and phase >= 0:
            fenetres[phase].append(tuple(map(int, m.groups()[2:])))
    for phase in range(3):
        if not fenetres[phase]:
            raise ValueError(f"releve C22:RSS manquant apres M{phase}")
        indicateurs[phase] = fenetres[phase][-1]
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
