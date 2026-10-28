#!/usr/bin/env python3
"""Une ligne de journal serie ENTIEREMENT ecrite qui correspond a un motif.

    ligne_complete.py JOURNAL REGEX              # code 0 si une telle ligne existe
    ligne_complete.py JOURNAL REGEX --derniere   # l'imprime (la derniere), code 1 sinon
    ligne_complete.py JOURNAL REGEX --toutes     # les imprime toutes, code 1 si aucune

P10_TELEMETRIE_COMPLETE_V1. QEMU ecrit la serie dans un fichier au fil de
l'eau ; un banc qui arrete QEMU des que le DEBUT d'une ligne apparait peut
le tuer au milieu de cette ligne. Run 40715886448 (77665751), scanout
virtio-gpu direct sous KVM : `[GUI-VIRTIO-GPU] ... us_par_present=250 ac`,
et quatre verifications qui lisent les champs suivants ont echoue.

Une ligne n'est complete que si elle est TERMINEE par un saut de ligne : le
dernier fragment d'un fichier sans `\\n` final est en cours d'ecriture, il
n'est jamais retenu. Le motif s'applique a la ligne sans couleurs ANSI ni
`\\r` final ; il doit donc nommer le dernier champ attendu (ancre `$`).
"""
import re
import sys

ANSI = re.compile(rb"\x1b\[[0-9;]*m")


def lignes_completes(donnees: bytes):
    morceaux = donnees.split(b"\n")
    # Le dernier morceau suit le dernier `\n` : vide si le fichier se termine
    # par un saut de ligne, sinon une ligne en cours d'ecriture.
    for brut in morceaux[:-1]:
        yield ANSI.sub(b"", brut).rstrip(b"\r").decode("utf-8", "replace")


def toutes(donnees: bytes, motif: str):
    expr = re.compile(motif)
    return [ligne for ligne in lignes_completes(donnees) if expr.search(ligne)]


def derniere(donnees: bytes, motif: str):
    retenues = toutes(donnees, motif)
    return retenues[-1] if retenues else None


def main(argv) -> int:
    if len(argv) not in (3, 4) or (len(argv) == 4 and argv[3] not in ("--derniere", "--toutes")):
        print(__doc__.strip().splitlines()[2], file=sys.stderr)
        return 2
    try:
        with open(argv[1], "rb") as f:
            donnees = f.read()
    except OSError:
        return 1
    retenues = toutes(donnees, argv[2])
    if not retenues:
        return 1
    if len(argv) == 4:
        print("\n".join(retenues if argv[3] == "--toutes" else retenues[-1:]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
