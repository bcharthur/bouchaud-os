#!/usr/bin/env python3
"""Interdit `[^\\r]` dans les expressions regulieres de `grep` et de Python.

BOUCHAUD_C59_LE_PIEGE_QUI_EST_REVENU_TROIS_FOIS

# Le defaut

Dans une expression reguliere POSIX -- celle de `grep`, de `sed`, et de `re`
en Python pour une chaine non brute -- `\\r` a l'interieur d'une classe ne
designe PAS un retour chariot. La classe contient les DEUX caracteres
litteraux `\\` et `r`. `[^\\r]*` signifie donc « tout sauf un antislash et
tout sauf la lettre r », et s'arrete au premier `r` du texte.

# Ce qu'il a coute

Trois fois, et a chaque fois un run complet :

    balayage de tranche   le tableau sortait vide, sans se plaindre
    bloc de preuves       `BACKING_PROBE path=/bo-navigateu`
    fin de session        `BOUCHAUD_SESSION_FIN etat=arretee BOUCHAUD_SYSTEM_EXIT`

La troisieme est la pire : la raison de l'extinction -- `autorun_termine` --
etait dans le journal, et le rapport l'a tue juste avant de l'afficher.

Corriger les occurrences ne suffit pas : elles sont revenues. Ce garde-fou
existe pour que la quatrieme ne compile pas.

# Ce qui reste autorise

`[^\\r\\n]` en JavaScript est CORRECT : les classes y interpretent bien les
echappements. `third_party/` n'est donc pas inspecte -- ce n'est pas notre
code, et les motifs y sont des regex JS.

Pour dire « jusqu'a la fin de la ligne » dans un `grep`, la reponse est `.*`,
et `tr -d '\\r'` pour retirer les retours chariot.
"""
import pathlib
import re
import sys

RACINE = pathlib.Path(__file__).resolve().parent.parent
EXCLUS = ("third_party/", "target/", ".git/")
SUFFIXES = (".sh", ".py", ".rs", ".bsh")
MOTIF = re.compile(r"\[\^\\r")


def pertinent(chemin):
    rel = str(chemin.relative_to(RACINE)).replace("\\", "/")
    if any(rel.startswith(e) for e in EXCLUS):
        return False
    return chemin.suffix in SUFFIXES


def main():
    fautes = []
    for chemin in sorted(RACINE.rglob("*")):
        if not chemin.is_file() or not pertinent(chemin):
            continue
        try:
            lignes = chemin.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError:
            continue
        for n, ligne in enumerate(lignes, 1):
            if not MOTIF.search(ligne):
                continue
            # Le fichier de garde et les commentaires qui DOCUMENTENT le piege
            # doivent pouvoir le citer, sinon on ne peut plus l'expliquer.
            depouille = ligne.lstrip()
            if chemin.name == pathlib.Path(__file__).name:
                continue
            if depouille.startswith("#") or depouille.startswith("//"):
                continue
            rel = chemin.relative_to(RACINE)
            fautes.append(f"  {rel}:{n}  {ligne.strip()[:90]}")

    if fautes:
        print("motif retour chariot : `[^\\r]` employe dans une regex POSIX",
              file=sys.stderr)
        for f in fautes:
            print(f, file=sys.stderr)
        print("", file=sys.stderr)
        print("  Dans grep/sed/re, `\\r` dans une classe vaut les DEUX lettres",
              file=sys.stderr)
        print("  `\\` et `r` : le motif s'arrete au premier `r` du texte.",
              file=sys.stderr)
        print("  Pour « jusqu'a la fin de ligne », ecrire `.*` puis `tr -d '\\r'`.",
              file=sys.stderr)
        return 1

    print("motif retour chariot : aucun `[^\\r]` dans une regex POSIX")
    return 0


if __name__ == "__main__":
    sys.exit(main())
