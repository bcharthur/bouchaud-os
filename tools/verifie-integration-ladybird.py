#!/usr/bin/env python3
"""Garde-fou : le rapport d'integration dit ce que le depot fait.

# Le defaut que cette garde previent

Un tableau d'avancement tenu a la main vieillit toujours dans le meme sens :
vers le haut. Une ligne verte qui cesse d'etre vraie ne coute rien a laisser,
et personne ne la relit. Au bout de quelques mois, le document affirme 95 %
parce que le binaire demarre.

`docs/ladybird/INTEGRATION_STATUS.md` est donc GENERE, et cette garde
verifie que le fichier commite est bien celui que la mesure produit
aujourd'hui. Une ligne qui passe au rouge fait echouer la CI ; une ligne
qu'on voudrait verte sans preuve ne peut pas y arriver.

    python3 tools/ladybird/mesure-integration.py --ecris

# Ce que cette garde NE verifie pas

Que le navigateur marche. Elle verifie que le document ne ment pas sur ce qui
a ete prouve -- ce qui est une garantie plus faible, et la seule qu'un
document puisse offrir.
"""

import re
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
DOCUMENT = RACINE / "docs/ladybird/INTEGRATION_STATUS.md"
MESURE = RACINE / "tools/ladybird/mesure-integration.py"

DEBUT = "<!-- MESURE:DEBUT -->"
FIN = "<!-- MESURE:FIN -->"


def bloc_de(texte, quoi):
    debut = texte.find(DEBUT)
    fin = texte.find(FIN, debut + 1)
    if debut < 0 or fin < 0:
        print("  - %s : les balises MESURE:DEBUT / MESURE:FIN ont disparu." % quoi)
        return None
    return texte[debut : fin + len(FIN)]


def main():
    if not DOCUMENT.exists():
        print("  - docs/ladybird/INTEGRATION_STATUS.md absent.")
        print("    le produire : python3 tools/ladybird/mesure-integration.py --ecris")
        return 1
    if not MESURE.exists():
        print("  - tools/ladybird/mesure-integration.py absent : le document "
              "ne serait plus reproductible.")
        return 1

    acheve = subprocess.run(
        [sys.executable, str(MESURE)], cwd=RACINE, capture_output=True, text=True, timeout=900
    )
    if acheve.returncode != 0:
        print("  - la mesure elle-meme a echoue :")
        for ligne in (acheve.stderr or acheve.stdout).strip().splitlines()[-5:]:
            print("      %s" % ligne)
        return 1

    fraiche = bloc_de(acheve.stdout, "la mesure")
    ecrit = bloc_de(DOCUMENT.read_text(encoding="utf-8"), "le document")
    if fraiche is None or ecrit is None:
        return 1

    if fraiche == ecrit:
        # Le chiffre, pour qu'il apparaisse dans le journal de CI : c'est la
        # seule ligne que quelqu'un lira sans ouvrir le document.
        resume = ""
        for ligne in fraiche.splitlines():
            if "prouve par execution" in ligne:
                resume = ligne.strip()
        print("integration ladybird : %s" % (resume or "document a jour"))
        return 0

    print("integration ladybird : le document n'est plus celui de la mesure.\n")
    lignes_ecrites = {l for l in ecrit.splitlines() if l.startswith("|")}
    lignes_fraiches = {l for l in fraiche.splitlines() if l.startswith("|")}
    for ligne in sorted(lignes_fraiches - lignes_ecrites):
        print("  mesure :  %s" % ligne)
    for ligne in sorted(lignes_ecrites - lignes_fraiches):
        print("  document : %s" % ligne)
    for ligne in fraiche.splitlines():
        if ligne.strip().startswith(("prouve", "cable", "vu sur", "non mesure", "en echec")):
            print("  %s" % ligne.strip())
    print("\n  corriger avec : python3 tools/ladybird/mesure-integration.py --ecris")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
