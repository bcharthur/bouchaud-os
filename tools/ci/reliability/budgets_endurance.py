#!/usr/bin/env python3
"""Verdict d'une tranche d'endurance : ce qui a tourne, ce qui a ete mesure.

# Pourquoi ce module existe

L'etape des budgets du workflow cherchait `cycle-*/serial.log` quand
`qemu_matrix.run_one` ecrivait `smp{N}.log`. Le motif ne correspondait a
rien, l'etape ecrivait « aucune trace serie produite » et tombait -- sans
jamais avoir lu une seule trace. Une campagne de quatre fois quatre-vingt-dix
minutes rendait donc un rouge qui ne disait rien du noyau.

Ce module ne DEVINE plus aucun nom : il lit `summary.json`, que `soak.py`
ecrit, et suit le champ `log` de chaque cycle. Le nom de la trace n'est connu
qu'a un seul endroit (`qemu_matrix.journal_du_cycle`), et
`test_traces_endurance.py` verifie le contrat de bout en bout.

# Quatre questions, quatre reponses separees

Un seul « rouge » melangeait des choses qui n'appellent pas la meme reaction :

  1. CAMPAGNE -- a-t-elle tourne ? cycles executes, cycles reussis, cycles
     tronques evites, code QEMU ;
  2. BUDGETS -- ont-ils ete EVALUES, trace par trace, par
     `check_budgets.py --journal` ? et avec quel resultat ?
  3. METRIQUES ABSENTES -- ce que le noyau n'a pas emis. Une grandeur requise
     absente est une faute (c'est `check_budgets.py` qui le decide), une
     grandeur optionnelle absente est ANNONCEE, jamais tue ;
  4. NOYAU -- panique, faute fatale, marqueur de famille manquant (famille
     figee ou en echec), statut de l'autorun.

Aucune de ces reponses n'est fabriquee ici : chaque chiffre vient de la trace
serie du vrai noyau, ou du resume que `soak.py` a ecrit en la lisant.

# Fail-closed

Resume absent, zero cycle, trace absente ou vide, cycle en echec, budget
depasse ou requis absent : le verdict est ROUGE. Rien d'absent ne se
transforme en succes.

    budgets_endurance.py DOSSIER_TRANCHE [--check-budgets CHEMIN]
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[3]
CHECK_BUDGETS = RACINE / "tools" / "ci" / "check_budgets.py"

# Ce que les familles de charge ecrivent. Relu ici pour le RAPPORT, jamais pour
# le verdict de campagne : celui-la est deja dans `summary.json`.
RESULTAT = re.compile(r"RESULTAT : (\d+) verification\(s\) en echec \((\d+)")
PIRE = re.compile(r"\[SCHED-NG-PIRE\][^\n]*")
FIN = re.compile(r"=== AUTORUN FIN === statut=(\d+)")
ABSENT_REQUIS = re.compile(r"^\s*(\S+) : REQUIS mais absent", re.M)
ABSENT_OPTIONNEL = re.compile(r"^\s*(\S+) : absent du journal", re.M)
MESUREES = re.compile(r"execution : (\d+)/(\d+) grandeur\(s\) mesuree\(s\)")
ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")


def trace_du_cycle(resume: Path, cycle: dict) -> Path | None:
    """La trace publiee par `soak.py`, la ou elle se trouve MAINTENANT.

    `soak.py` ecrit le chemin tel qu'il l'a ouvert, souvent relatif au
    repertoire de travail du job. Apres un telechargement d'artefact, seul le
    suffixe `cycle-NNNN/<nom>` reste valable : on le cherche sous le dossier
    du resume. Le NOM n'est jamais reconstruit -- il vient du resume.
    """
    publie = cycle.get("log")
    if not publie:
        return None
    chemin = Path(publie)
    candidats = [chemin] if chemin.is_absolute() else [Path.cwd() / chemin]
    if len(chemin.parts) >= 2:
        candidats.append(resume.parent / chemin.parts[-2] / chemin.parts[-1])
    for candidat in candidats:
        if candidat.is_file():
            return candidat
    return None


def evalue_budgets(trace: Path, check_budgets: Path) -> dict:
    """Execute `check_budgets.py --journal` sur UNE trace et garde sa sortie."""
    fini = subprocess.run(
        [sys.executable, str(check_budgets), "--journal", str(trace)],
        capture_output=True, text=True, check=False,
    )
    sortie = fini.stdout + fini.stderr
    mesurees = MESUREES.search(sortie)
    return {
        "rc": fini.returncode,
        "requis_absents": ABSENT_REQUIS.findall(sortie),
        "optionnels_absents": ABSENT_OPTIONNEL.findall(sortie),
        "mesurees": int(mesurees.group(1)) if mesurees else None,
        "budgets": int(mesurees.group(2)) if mesurees else None,
        "sortie": sortie,
    }


def releve_noyau(trace: Path) -> dict:
    texte = ANSI.sub("", trace.read_text(encoding="utf-8", errors="replace"))
    fin = FIN.findall(texte)
    return {
        "autorun_statut": int(fin[-1]) if fin else None,
        "resultats_sondes": [
            {"echecs": int(e), "passees": int(t)} for e, t in RESULTAT.findall(texte)
        ],
        "pire_attente": PIRE.findall(texte),
    }


def juge(dossier: Path, check_budgets: Path) -> dict:
    resume = dossier / "summary.json"
    fautes: list[str] = []
    rapport: dict = {"dossier": str(dossier), "fautes": fautes}

    if not resume.is_file():
        fautes.append(f"campagne : {resume} absent -- soak.py n'a pas tourne jusqu'au bout")
        rapport["ok"] = False
        return rapport
    donnees = json.loads(resume.read_text(encoding="utf-8"))
    cycles = donnees.get("cycles", [])
    reussis = sum(1 for c in cycles if c.get("ok"))
    rapport["campagne"] = {
        "cycles": len(cycles),
        "reussis": reussis,
        "tronques_evites": donnees.get("cycles_tronques_evites"),
        "cpus": donnees.get("cpus"),
        "disque": donnees.get("disque"),
        "marqueurs_exiges": len(donnees.get("marqueurs_exiges", [])),
    }
    if not cycles:
        fautes.append("campagne : zero cycle execute -- rien n'a ete mesure")
    if donnees.get("disque") is None:
        # Sans disque de scenario, le noyau demarre et attend : aucun releve,
        # aucune charge. Ce n'est pas une endurance.
        fautes.append("campagne : aucun disque de scenario attache")

    rapport["cycles"] = []
    for index, cycle in enumerate(cycles, 1):
        fiche: dict = {
            "index": index,
            "ok": bool(cycle.get("ok")),
            "log_publie": cycle.get("log"),
            "qemu_rc": cycle.get("returncode"),
            "timeout": cycle.get("timed_out"),
            "fatals": cycle.get("fatal_findings", []),
            "marqueurs_absents": cycle.get("required_markers_missing", []),
        }
        rapport["cycles"].append(fiche)
        if not fiche["ok"]:
            fautes.append(
                f"noyau : cycle {index} en echec "
                f"({len(fiche['fatals'])} faute(s) fatale(s), "
                f"{len(fiche['marqueurs_absents'])} marqueur(s) absent(s))"
            )
        trace = trace_du_cycle(resume, cycle)
        if trace is None or trace.stat().st_size == 0:
            fautes.append(
                f"campagne : trace du cycle {index} introuvable ou vide "
                f"(publiee : {cycle.get('log')!r})"
            )
            fiche["budgets"] = None
            continue
        fiche["trace"] = str(trace)
        fiche["noyau"] = releve_noyau(trace)
        budgets = evalue_budgets(trace, check_budgets)
        fiche["budgets"] = {k: v for k, v in budgets.items() if k != "sortie"}
        fiche["budgets_sortie"] = budgets["sortie"]
        if budgets["rc"] != 0:
            fautes.append(
                f"budgets : cycle {index} rc={budgets['rc']} "
                f"({len(budgets['requis_absents'])} requis absent(s))"
            )

    rapport["ok"] = not fautes
    return rapport


def texte(rapport: dict) -> str:
    lignes = [f"== endurance : {rapport['dossier']} =="]
    campagne = rapport.get("campagne")
    if campagne:
        lignes.append(
            f"1. CAMPAGNE     : {campagne['cycles']} cycle(s) execute(s), "
            f"{campagne['reussis']} reussi(s), SMP{campagne['cpus']}, "
            f"{campagne['marqueurs_exiges']} marqueur(s) exige(s), "
            f"tronques evites={campagne['tronques_evites']}"
        )
    for fiche in rapport.get("cycles", []):
        b = fiche.get("budgets")
        if b is None:
            lignes.append(f"2. BUDGETS c{fiche['index']:<3}: NON EVALUES (trace absente)")
        else:
            lignes.append(
                f"2. BUDGETS c{fiche['index']:<3}: evalues rc={b['rc']} "
                f"mesurees={b['mesurees']}/{b['budgets']}"
            )
            lignes.append(
                f"3. ABSENTES c{fiche['index']:<2}: requises="
                + (",".join(b["requis_absents"]) or "aucune")
                + " ; optionnelles (annoncees, non verifiees)="
                + (",".join(b["optionnels_absents"]) or "aucune")
            )
        noyau = fiche.get("noyau", {})
        lignes.append(
            f"4. NOYAU c{fiche['index']:<5}: ok={fiche['ok']} fatals={len(fiche['fatals'])} "
            f"marqueurs_absents={fiche['marqueurs_absents']} "
            f"autorun_statut={noyau.get('autorun_statut')}"
        )
        for r in noyau.get("resultats_sondes", []):
            lignes.append(f"     sonde : {r['echecs']} echec(s), {r['passees']} passee(s)")
        for p in noyau.get("pire_attente", []):
            lignes.append(f"     {p}")
        if fiche.get("budgets_sortie") and fiche["budgets"]["rc"] != 0:
            lignes.extend("     | " + l for l in fiche["budgets_sortie"].rstrip().splitlines())
    lignes.append("VERDICT : " + ("OK" if rapport["ok"] else "ECHEC"))
    lignes.extend("  - " + f for f in rapport["fautes"])
    return "\n".join(lignes)


def main() -> int:
    parseur = argparse.ArgumentParser()
    parseur.add_argument("dossier", type=Path, help="dossier --out-dir de soak.py")
    parseur.add_argument("--check-budgets", type=Path, default=CHECK_BUDGETS)
    options = parseur.parse_args()

    rapport = juge(options.dossier, options.check_budgets)
    options.dossier.mkdir(parents=True, exist_ok=True)
    (options.dossier / "verdict-endurance.json").write_text(
        json.dumps(rapport, indent=2) + "\n", encoding="utf-8")
    rendu = texte(rapport)
    print(rendu)
    resume_gh = os.environ.get("GITHUB_STEP_SUMMARY")
    if resume_gh:
        with open(resume_gh, "a", encoding="utf-8") as f:
            f.write("```\n" + rendu + "\n```\n")
    return 0 if rapport["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
