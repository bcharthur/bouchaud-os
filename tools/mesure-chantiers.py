#!/usr/bin/env python3
"""Les chiffres des douze chantiers, MESURES sur le code, jamais recopies.

`docs/AUDIT_12_CHANTIERS.md` a peri exactement comme perissent tous les
documents qui portent des chiffres a la main : le code a avance, le document est
reste. Il affirmait « futex reste sous BKL » et « pas de W^X » alors que le
code disait l'inverse depuis plusieurs commits.

Un document faux est pire qu'un document absent : on lui fait confiance.

Ce script est la seule source de ces chiffres. Il les compte sur l'arbre, et
`--verifie` refuse un document qui ne les porte plus -- ce qui fait de la mise
a jour du document une condition de la barriere, et non une intention.
"""

import argparse
import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
AUDIT = RACINE / "docs/AUDIT_12_CHANTIERS.md"

# Les marqueurs que le document doit porter, et la mesure qui les remplit.
DEBUT = "<!-- MESURE-CHANTIERS:DEBUT -->"
FIN = "<!-- MESURE-CHANTIERS:FIN -->"


def appels_liberes() -> tuple:
    """(liberes, aiguilles) d'apres la table de politique et l'aiguillage."""
    bkl = (RACINE / "src/compat/linux/bkl.rs").read_text(encoding="utf-8")
    libres = set(re.findall(r"\(nr::(\w+),", bkl))
    nr = (RACINE / "src/compat/linux/nr.rs").read_text(encoding="utf-8")
    noms = set(re.findall(r"pub const (\w+)\s*:\s*u64\s*=", nr))
    mod = (RACINE / "src/compat/linux/mod.rs").read_text(encoding="utf-8")
    corps = mod[mod.index("fn dispatch("):]

    aiguilles = set(re.findall(r"\bnr::(\w+)\b", corps))
    # Les bras GROUPES comptent chacun pour un appel : `GETUID | GETEUID |
    # GETGID | GETEGID => 0` en aiguille quatre. Ne lire que le premier nom
    # avant `=>` en perdait vingt, et le document aurait annonce moins d'appels
    # liberes qu'il n'y en a -- une sous-estimation est un mensonge comme un
    # autre.
    for gauche in re.findall(r"^\s+([A-Z0-9_ |]+?)\s*=>", corps, re.M):
        for nom in (partie.strip() for partie in gauche.split("|")):
            if nom in noms:
                aiguilles.add(nom)
    aiguilles &= noms
    return len(aiguilles & libres), len(aiguilles)


def fichiers_avec_points_surs() -> int:
    preempt = RACINE / "src/kernel/scheduler/preempt.rs"
    total = 0
    for chemin in RACINE.glob("src/**/*.rs"):
        if chemin == preempt:
            continue
        for ligne in chemin.read_text(encoding="utf-8", errors="replace").splitlines():
            nu = ligne.strip()
            if nu.startswith("//"):
                continue
            if re.search(r"\bsafe_point\(\)", nu):
                total += 1
                break
    return total


def compte(motif: str) -> int:
    return len(list(RACINE.glob(motif)))


def wx_present() -> bool:
    """W^X est-il refuse a la source, et pas seulement declare ?"""
    wx = RACINE / "src/kernel/security/wx.rs"
    elf = RACINE / "src/kernel/process/elf.rs"
    return wx.exists() and "W^X" in elf.read_text(encoding="utf-8")


def canari_present() -> bool:
    return "CANARI_PILE" in (RACINE / "src/kernel/process/thread/tache.rs").read_text(
        encoding="utf-8"
    )


def mesures() -> list:
    liberes, aiguilles = appels_liberes()
    restants = aiguilles - liberes
    return [
        ("Appels systeme aiguilles", str(aiguilles)),
        ("Appels hors gros verrou", str(liberes)),
        ("Appels encore sous gros verrou", str(restants)),
        ("Fichiers portant un point sur de preemption", str(fichiers_avec_points_surs())),
        ("Garde-fous d'architecture", str(compte("tools/verifie-*.py"))),
        ("Suites de test hote", str(compte("tools/**/test_*.rs"))),
        ("W^X applique au chargement ELF", "oui" if wx_present() else "non"),
        ("Canari de pile noyau", "oui" if canari_present() else "non"),
    ]


def rendu() -> str:
    lignes = ["| Mesure | Valeur |", "|---|---|"]
    lignes += ["| %s | **%s** |" % (nom, valeur) for nom, valeur in mesures()]
    return "\n".join(lignes)


def main() -> int:
    parseur = argparse.ArgumentParser()
    parseur.add_argument("--verifie", action="store_true",
                         help="echoue si le document ne porte pas les mesures courantes")
    parseur.add_argument("--ecris", action="store_true",
                         help="reecrit le bloc mesure dans le document")
    options = parseur.parse_args()

    table = rendu()

    if not options.verifie and not options.ecris:
        print(table)
        return 0

    if not AUDIT.exists():
        print("  - document absent : %s" % AUDIT.relative_to(RACINE).as_posix())
        return 1

    texte = AUDIT.read_text(encoding="utf-8")
    if DEBUT not in texte or FIN not in texte:
        print("audit : les marqueurs %s / %s sont absents du document." % (DEBUT, FIN))
        return 1

    avant = texte[: texte.index(DEBUT) + len(DEBUT)]
    apres = texte[texte.index(FIN):]
    neuf = avant + "\n" + table + "\n" + apres

    if options.ecris:
        AUDIT.write_text(neuf, encoding="utf-8")
        print("audit : bloc mesure reecrit")
        return 0

    if neuf != texte:
        print("audit : les chiffres du document ne sont plus ceux du code.\n")
        print("  Attendu :\n")
        for ligne in table.splitlines():
            print("    " + ligne)
        print("\n  Corriger avec : python3 tools/mesure-chantiers.py --ecris\n")
        return 1

    print("audit : les chiffres du document sont ceux du code")
    return 0


if __name__ == "__main__":
    sys.exit(main())
