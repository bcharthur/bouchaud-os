#!/usr/bin/env python3
"""Verifie la politique ASCII des scripts PowerShell suivis par Git.

Sans argument :
    python3 tools/verifie-scripts-windows.py
verifie tous les .ps1 suivis par Git.

Avec --files-from :
    python3 tools/verifie-scripts-windows.py --files-from .ci-powershell-files
verifie uniquement les scripts listes. Cette forme est utilisee par CI Fast pour
ne bloquer une PR que sur les regressions PowerShell qu'elle introduit.

La regle ASCII reste volontairement stricte pour Windows PowerShell 5.1 : un
.ps1 UTF-8 sans BOM peut etre lu en Windows-1252, et certaines sequences UTF-8
deviennent alors des guillemets typographiques acceptes par l'analyseur.
"""

import argparse
import os
import subprocess
import sys
import unicodedata

RACINE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def scripts_suivis():
    """Tous les .ps1 suivis par Git, chemins relatifs normalises."""
    sortie = subprocess.run(
        ["git", "-C", RACINE, "ls-files", "*.ps1"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return sorted(ligne.replace("\\", "/") for ligne in sortie.splitlines() if ligne.strip())


def scripts(demandes=None):
    """Filtre optionnellement la liste suivie par une liste de chemins."""
    suivis = scripts_suivis()
    if demandes is None:
        return suivis

    demandes = {
        chemin.strip().replace("\\", "/")
        for chemin in demandes
        if chemin and chemin.strip()
    }
    return [chemin for chemin in suivis if chemin in demandes]


def fautes(chemin):
    """Caracteres non ASCII d'un fichier, avec ligne et nom Unicode."""
    with open(os.path.join(RACINE, chemin), "rb") as fichier:
        octets = fichier.read()

    if octets.startswith(b"\xef\xbb\xbf"):
        return [(1, "\ufeff", "MARQUE D'ORDRE DES OCTETS (BOM)")]

    try:
        texte = octets.decode("utf-8")
    except UnicodeDecodeError as exc:
        return [(0, "?", "fichier illisible en UTF-8 : %s" % exc)]

    trouvees = []
    for numero, ligne in enumerate(texte.splitlines(), start=1):
        for caractere in ligne:
            if ord(caractere) > 127:
                try:
                    nom = unicodedata.name(caractere)
                except ValueError:
                    nom = "U+%04X" % ord(caractere)
                trouvees.append((numero, caractere, nom))
    return trouvees


def lire_demandes(chemin):
    with open(chemin, encoding="utf-8") as fichier:
        return [ligne.rstrip("\r\n") for ligne in fichier]


def principal():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--files-from",
        metavar="FICHIER",
        help="ne verifier que les chemins listes, un par ligne",
    )
    parser.add_argument(
        "paths",
        nargs="*",
        help="chemins .ps1 a verifier (optionnel)",
    )
    args = parser.parse_args()

    demandes = None
    mode_filtre = bool(args.files_from or args.paths)
    if args.files_from:
        demandes = lire_demandes(args.files_from)
    if args.paths:
        demandes = (demandes or []) + args.paths

    fichiers = scripts(demandes if mode_filtre else None)

    # En mode filtre, zero .ps1 est un resultat legitime : la PR n'en touche pas.
    if not fichiers:
        if mode_filtre:
            print("aucun script PowerShell suivi a verifier dans ce changement.")
            return 0
        print("aucun script PowerShell trouve - la verification ne prouve rien")
        return 1

    total = 0
    for chemin in fichiers:
        trouvees = fautes(chemin)
        if not trouvees:
            print("  ascii  %s" % chemin)
            continue

        total += len(trouvees)
        print("  ECHEC  %s" % chemin)
        for numero, caractere, nom in trouvees[:10]:
            print("           ligne %-4d %-3r %s" % (numero, caractere, nom))
        if len(trouvees) > 10:
            print("           ... et %d autre(s)" % (len(trouvees) - 10))

    print("")
    if total:
        print("%d caractere(s) non-ASCII dans les scripts PowerShell verifies." % total)
        print("Windows PowerShell 5.1 peut les decoder avec une page de codes ANSI.")
        return 1

    print("%d script(s) PowerShell verifies, tous en ASCII strict." % len(fichiers))
    return 0


if __name__ == "__main__":
    sys.exit(principal())
