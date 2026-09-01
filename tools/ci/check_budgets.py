#!/usr/bin/env python3
"""Budgets versionnes : ce qui a ete gagne ne doit pas se reperdre.

# Pourquoi des budgets, et pas seulement des tests

Un test dit « cela marche ». Il ne dit pas « cela ne s'est pas degrade ». Or
les regressions qui comptent ici sont graduelles : un site d'acquisition du
gros verrou rajoute dans un sous-systeme qu'on venait d'alleger, une tenue
maximale qui remonte de dix millisecondes par semaine. Aucune n'echoue a un
test ; toutes annulent le travail, et on ne s'en apercoit qu'au moment ou la
machine rame de nouveau.

Un budget est une valeur de reference VERSIONNEE. Le depasser echoue la
barriere ; faire mieux est signale, et s'adopte explicitement.

# Les deux familles

  * ARCHITECTURE -- se calcule sur la SOURCE, donc partout et tout de suite :
    combien de sites prennent encore le gros verrou, et dans quel domaine.
    C'est la mesure directe du chantier « sortie du gros verrou ».

  * EXECUTION -- se lit dans un journal QEMU (`--journal`). Tenue maximale,
    attente maximale, regressions de domaine. Sans journal, ces budgets sont
    ANNONCES COMME NON VERIFIES et non silencieusement reussis : un budget
    qu'on croit vert alors qu'il n'a pas tourne est pire que pas de budget.
"""

import argparse
import json
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent.parent
REFERENCE = Path(__file__).resolve().parent / "budgets" / "bkl-sites.json"
SRC = RACINE / "src"

ACQUISITION = re.compile(r"\bsmp_lock::(?:enter|try_enter|try_enter_depuis_zero)\(\)")
PORTEE = re.compile(r"\bportee\(\s*(?:crate::kernel::sync::)?Domaine::(\w+)")

EXEMPTS_PREFIXES = ("src/kernel/sync/bkl/",)
EXEMPTS = {"src/kernel/sync/bkl.rs", "src/kernel/sync/mod.rs"}


def sites_par_domaine() -> dict[str, int]:
    """Combien de sites prennent le gros verrou, et sous quel domaine."""
    compte: dict[str, int] = {}
    for chemin in sorted(SRC.rglob("*.rs")):
        relatif = chemin.relative_to(RACINE).as_posix()
        if relatif in EXEMPTS or relatif.startswith(EXEMPTS_PREFIXES):
            continue
        lignes = chemin.read_text(encoding="utf-8", errors="replace").split("\n")
        for numero, ligne in enumerate(lignes):
            if ligne.lstrip().startswith("//") or not ACQUISITION.search(ligne):
                continue
            domaine = "Indetermine"
            for precedente in reversed(lignes[max(0, numero - 3):numero]):
                trouve = PORTEE.search(precedente)
                if trouve:
                    domaine = trouve.group(1)
                    break
            compte[domaine] = compte.get(domaine, 0) + 1
    return compte


# Les grandeurs lues dans un journal d'execution, et leur sens.
EXECUTION = {
    "bkl_max_hold_ms": (
        re.compile(r"\[BKL-MAX-HOLD\]\s+ns=(\d+)"),
        lambda v: v / 1_000_000,
        "plus longue tenue du gros verrou",
    ),
    "bkl_attente_max_ms": (
        re.compile(r"\[BKL-COMPTES\].*?attente_max_ns=(\d+)"),
        lambda v: v / 1_000_000,
        "plus longue attente avant acquisition",
    ),
    "bkl_regressions_domaine": (
        re.compile(r"\[BKL-DOMAINES\]\s+normaux=\d+\s+regressions=(\d+)"),
        lambda v: v,
        "chemins declares sortis ayant repris le verrou",
    ),
}


def mesures_execution(journal: Path) -> dict[str, float]:
    texte = journal.read_text(encoding="utf-8", errors="replace")
    mesures = {}
    for nom, (motif, converti, _) in EXECUTION.items():
        valeurs = [converti(float(m)) for m in motif.findall(texte)]
        if valeurs:
            # Le PIRE de la trace : un budget se juge sur le maximum, pas sur
            # une moyenne qui noierait justement le figement qu'on cherche.
            mesures[nom] = max(valeurs)
    return mesures


def main() -> int:
    parseur = argparse.ArgumentParser()
    parseur.add_argument("--journal", type=Path,
                         help="journal serie QEMU, pour les budgets d'execution")
    parseur.add_argument("--adopte", action="store_true",
                         help="ecrit les valeurs courantes comme nouvelle reference")
    options = parseur.parse_args()

    reference = json.loads(REFERENCE.read_text(encoding="utf-8"))
    courant = sites_par_domaine()

    if options.adopte:
        reference["architecture"]["sites_bkl_par_domaine"] = courant
        if options.journal and options.journal.exists():
            reference["execution"].update(mesures_execution(options.journal))
        REFERENCE.write_text(json.dumps(reference, indent=2, sort_keys=True) + "\n",
                             encoding="utf-8")
        print(f"reference adoptee : {REFERENCE.relative_to(RACINE)}")
        return 0

    fautes, gains, non_verifies = [], [], []

    # --- architecture -------------------------------------------------------
    attendu = {c: v for c, v in
               reference["architecture"]["sites_bkl_par_domaine"].items()
               if not c.startswith("_")}
    for domaine in sorted(set(attendu) | set(courant)):
        budget = attendu.get(domaine, 0)
        mesure = courant.get(domaine, 0)
        if mesure > budget:
            fautes.append(
                f"  sites BKL / {domaine} : {mesure} > {budget} (budget). "
                f"Un site rajoute dans un sous-systeme qu'on allege annule le "
                f"travail sans echouer a aucun test."
            )
        elif mesure < budget:
            gains.append(f"  sites BKL / {domaine} : {mesure} < {budget}")

    # --- execution ----------------------------------------------------------
    if options.journal and options.journal.exists():
        mesures = mesures_execution(options.journal)
        for nom, budget in reference["execution"].items():
            if nom.startswith("_"):
                continue
            if nom not in mesures:
                non_verifies.append(f"  {nom} : absent du journal")
                continue
            mesure = mesures[nom]
            libelle = EXECUTION[nom][2] if nom in EXECUTION else nom
            if mesure > budget:
                fautes.append(f"  {nom} ({libelle}) : {mesure:.3f} > {budget} (budget)")
            elif mesure < budget:
                gains.append(f"  {nom} : {mesure:.3f} < {budget}")
    else:
        non_verifies = [f"  {nom} : aucun journal fourni"
                        for nom in sorted(reference["execution"])
                        if not nom.startswith("_")]

    if gains:
        print("budgets ameliores (adopter avec --adopte) :")
        print("\n".join(gains))
    if non_verifies:
        # Annonce, jamais silence : un budget qu'on croit vert alors qu'il n'a
        # pas tourne est pire que pas de budget du tout.
        print("budgets NON VERIFIES (pas de mesure disponible) :")
        print("\n".join(non_verifies))
    if fautes:
        print("budgets depasses :")
        print("\n".join(fautes))
        return 1

    total = sum(courant.values())
    print(f"ok  budgets tenus ; {total} site(s) d'acquisition du gros verrou "
          f"dans {len(courant)} domaine(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
