#!/usr/bin/env python3
"""Qualification STRICTE P13 sur traces reelles, distincte du succes du banc.

Usage : qualifie_p13.py attribution|ab JOURNAL --smp N [--passes N] [--mode strict|report]

'report' informe sans bloquer les tests fonctionnels ; 'strict' exige une
preuve valable pour une baseline et retourne non-zero sinon.

Ne traite jamais NON_QUALIFIANT / NON_CONCLUANT / NON_CONVAINCANT comme des verts.
"""
from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

from analyse_ab_reveil_precis import analyse, BRAS, FIN_BRAS

PREFIXE = re.compile(r"^(?:\[[^\]]*\]){2}\[FPS:[^\]]*\] ")
ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
SONDE = re.compile(r"^VERDICT_SONDE : (\S+)")
COMPTE_HOTE = re.compile(r"^\s+comptes [^:]+ : preuve_hote=(\S+)")
MINUTEUR = re.compile(r"REVEIL_PRECIS voulu=([01]) minuteur=(\S+) armes=")


def lignes_invite(texte: str) -> list[str]:
    """Retire les prefixes de l'hote sans transformer un echo shell en preuve."""
    lignes = []
    for brute in texte.splitlines():
        l = PREFIXE.sub("", ANSI.sub("", brute.rstrip("\r")))
        lignes.append(l)
    return lignes


def inspecte_attribution(texte: str, smp: int, passes: int) -> list[str]:
    ls = lignes_invite(texte)
    fautes = []
    if any("KERNEL PANIC" in l or "panicked at" in l for l in ls):
        fautes.append("panique noyau")
    if not any(l.strip() == "ORDO_ATTRIBUTION_FIN" for l in ls):
        fautes.append("marqueur ORDO_ATTRIBUTION_FIN execute absent")
    verdicts = [m.group(1) for l in ls if (m := SONDE.match(l))]
    if len(verdicts) != passes:
        fautes.append(f"verdicts {len(verdicts)}/{passes}")
    for i, verdict in enumerate(verdicts, 1):
        if verdict != "QUALIFIE_OK":
            fautes.append(f"passe {i} : {verdict} (attendu QUALIFIE_OK)")
    preuves = [m.group(1) for l in ls if (m := COMPTE_HOTE.match(l))]
    if len(preuves) < passes * 3:
        fautes.append(f"phases instrumentees {len(preuves)}/{passes * 3}")
    if any(p != "kvm-steal" for p in preuves):
        fautes.append("une ou plusieurs phases sans preuve hote kvm-steal")
    # 'kvm-steal' n'a de sens que dans les journaux produits sous KVM.
    if smp not in (1, 2, 4):
        fautes.append(f"SMP{smp} hors perimetre baseline stricte 1/2/4")
    return fautes


def inspecte_ab(texte: str, smp: int, tours: int) -> list[str]:
    ls = lignes_invite(texte)
    fautes = []
    if not any(l.strip() == "AB_PRECIS_FIN" for l in ls):
        fautes.append("marqueur AB_PRECIS_FIN execute absent")
    rapport, verdict, code = analyse(texte, tours, str(smp), "kvm")
    if code != 0 or verdict != "CONVAINCANT":
        fautes.append(f"A/B = {verdict} (attendu CONVAINCANT sans defaut)")
        fautes.extend(s.strip() for s in rapport if s.strip().startswith(("DEFAUT", "ECHOUE")))
    # Deux bras par tour. La qualification est obligatoire meme lorsque les
    # valeurs de latence ont l'air favorables.
    v = [m.group(1) for l in ls if (m := SONDE.match(l))]
    if len(v) != 2 * tours:
        fautes.append(f"verdicts par bras {len(v)}/{2 * tours}")
    for i, statut in enumerate(v, 1):
        if statut != "QUALIFIE_OK":
            fautes.append(f"bras {i} : {statut} (attendu QUALIFIE_OK)")
    # The autorun intentionally restores the default policy after the last arm.
    # That REVEIL_PRECIS line is not a measurement of any arm. Qualify exactly
    # the two counter snapshots delimited by each executed AB_PRECIS/FIN pair,
    # never a global count that can reject a valid restoration (17 vs 16).
    bras_minuteurs = []
    courant = None
    for ligne in ls:
        debut = BRAS.match(ligne)
        if debut:
            courant = [int(debut.group(1)), debut.group(2), []]
            bras_minuteurs.append(courant)
            continue
        if FIN_BRAS.match(ligne):
            courant = None
            continue
        releve = MINUTEUR.search(ligne) if courant is not None else None
        if releve:
            courant[2].append((int(releve.group(1)), releve.group(2)))
    if len(bras_minuteurs) != 2 * tours:
        fautes.append(f"bras minuteur identifies {len(bras_minuteurs)}/{2 * tours}")
    for tour, mode, releves in bras_minuteurs:
        attendu = 1 if mode == "on" else 0
        if len(releves) != 2:
            fautes.append(f"tour {tour} {mode} : releves minuteur {len(releves)}/2")
        elif any(voulu != attendu or timer != "tsc-deadline" for voulu, timer in releves):
            fautes.append(f"tour {tour} {mode} : minuteur KVM TSC-deadline ou politique non prouves")
    if smp not in (1, 4):
        fautes.append(f"SMP{smp} hors perimetre A/B strict 1/4")
    return fautes


def juge(genre: str, journal: Path, smp: int, passes: int) -> list[str]:
    if not journal.is_file() or journal.stat().st_size == 0:
        return [f"journal absent ou vide : {journal}"]
    if f"kvm-smp{smp}" not in journal.parent.name:
        return [f"provenance SMP/acceleration incoherente : {journal.parent.name} (attendu *kvm-smp{smp})"]
    texte = journal.read_text(encoding="utf-8", errors="replace")
    if genre == "attribution":
        return inspecte_attribution(texte, smp, passes)
    if genre == "ab":
        return inspecte_ab(texte, smp, passes)
    return [f"genre inconnu {genre}"]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("genre", choices=("attribution", "ab"))
    ap.add_argument("journal", type=Path)
    ap.add_argument("--smp", type=int, required=True)
    ap.add_argument("--passes", type=int, default=None, help="2 pour attribution, 4 tours pour A/B")
    ap.add_argument("--mode", choices=("strict", "report"), default="strict")
    ns = ap.parse_args()
    passes = ns.passes if ns.passes is not None else (2 if ns.genre == "attribution" else 4)
    if passes <= 0:
        ap.error("--passes doit etre positif")
    fautes = juge(ns.genre, ns.journal, ns.smp, passes)
    titre = f"P13_{'STRICT_OK' if not fautes else 'BASELINE_NON_ETABLIE'} genre={ns.genre} smp={ns.smp}"
    print(titre)
    for f in fautes:
        print(f"  - {f}")
    resume = os.environ.get("GITHUB_STEP_SUMMARY")
    if resume:
        with open(resume, "a", encoding="utf-8") as h:
            h.write(f"### {titre}\n")
            for f in fautes:
                h.write(f"- {f}\n")
            h.write("\n")
    return int(bool(fautes) and ns.mode == "strict")


if __name__ == "__main__":
    sys.exit(main())
