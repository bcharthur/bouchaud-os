#!/usr/bin/env python3
"""Symbolise un RIP releve par la BLACKBOX, contre le noyau EXACT qui l'a produit.

# Pourquoi cet outil existe

Un releve de faute physique donne une adresse :

    DOUBLE FAULT vector=8 cpu=0 task=desktop RIP=0x80012ed45c

Cette adresse ne veut rien dire sans le noyau qui l'a produite. La resoudre
contre le HEAD courant donne une reponse -- et c'est presque toujours la
MAUVAISE : entre deux commits, l'editeur de liens deplace tout. Une fonction
innocente prend la place de la coupable, et l'enquete part dans la mauvaise
direction avec une confiance parfaitement injustifiee.

C'est arrive dans ce depot : `0x80012ed45c` resolu contre un HEAD posterieur
designait `keyboard::read_into`, qui n'a rien a voir avec le chemin de
stockage ou la machine est morte.

# Ce que l'outil garantit

Il refuse de repondre si le noyau qu'on lui donne n'est pas celui qui a produit
l'adresse. Le manifeste ecrit a la construction de l'image porte le SHA256 du
noyau ; l'outil le recalcule et compare. Une reponse rendue est donc une
reponse sur le bon binaire, ou il n'y a pas de reponse.

# Usage

    python3 tools/reference/symbolise-blackbox.py 0x80012ed45c \\
        --manifeste target/reference/bouchaud-trigkey.manifeste.json

    python3 tools/reference/symbolise-blackbox.py 0x80012ed45c \\
        --noyau target/x86_64-bouchaud_os_uefi/debug/bouchaud-os --base 0x8000000000

Plusieurs adresses peuvent etre passees d'un coup : une trame complete se
symbolise en une commande.
"""

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[2]

# Base de chargement du noyau PIE UEFI. Le manifeste la porte ; cette valeur
# n'est qu'un defaut pour les invocations a la main.
BASE_DEFAUT = 0x8000000000


def sha256(chemin: Path) -> str:
    h = hashlib.sha256()
    with open(chemin, "rb") as f:
        for bloc in iter(lambda: f.read(1 << 20), b""):
            h.update(bloc)
    return h.hexdigest()


def outil(*noms):
    for nom in noms:
        chemin = shutil.which(nom)
        if chemin:
            return chemin
    return None


def symbolise(noyau: Path, offsets):
    """(fonction, fichier:ligne) par offset, au mieux de ce que porte l'ELF."""
    addr2line = outil("llvm-addr2line", "addr2line")
    resultats = {}
    if addr2line:
        # UNE ADRESSE A LA FOIS, ET SANS `-i`.
        #
        # Avec `-i`, `addr2line` rend un nombre VARIABLE de lignes par adresse
        # -- une paire par etage d'inlining. Un decoupage deux par deux
        # decalerait alors toutes les reponses suivantes, et rendrait des
        # symboles faux avec l'apparence de symboles justes. Sans `-i`, le
        # format est exactement deux lignes.
        for offset in offsets:
            sortie = subprocess.run(
                [addr2line, "-f", "-C", "-e", str(noyau), hex(offset)],
                capture_output=True, text=True,
            )
            lignes = sortie.stdout.splitlines()
            if sortie.returncode == 0 and len(lignes) >= 2 and lignes[0] != "??":
                resultats[offset] = (lignes[0], lignes[1])
    if len(resultats) == len(offsets):
        return resultats

    # Repli : la table des symboles. Moins precis -- pas de ligne -- mais il
    # repond quand l'ELF n'a pas de DWARF.
    nm = outil("llvm-nm", "nm")
    if not nm:
        return resultats
    sortie = subprocess.run([nm, "-C", "--defined-only", "-n", str(noyau)],
                            capture_output=True, text=True)
    symboles = []
    for ligne in sortie.stdout.splitlines():
        morceaux = ligne.split(None, 2)
        if len(morceaux) < 3:
            continue
        try:
            adresse = int(morceaux[0], 16)
        except ValueError:
            continue
        symboles.append((adresse, morceaux[2]))
    symboles.sort()
    for offset in offsets:
        if offset in resultats:
            continue
        choisi = None
        for adresse, nom in symboles:
            if adresse <= offset:
                choisi = (adresse, nom)
            else:
                break
        if choisi:
            resultats[offset] = ("%s+0x%x" % (choisi[1], offset - choisi[0]), "??:0")
    return resultats


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("adresses", nargs="+", help="RIP releves, en hexadecimal")
    p.add_argument("--manifeste", type=Path,
                   help="manifeste ecrit a la construction de l'image")
    p.add_argument("--noyau", type=Path, help="ELF noyau non strip")
    p.add_argument("--base", help="base de chargement (defaut : celle du manifeste)")
    p.add_argument("--sans-verification", action="store_true",
                   help="repondre meme si le noyau ne correspond pas au manifeste")
    args = p.parse_args()

    noyau = args.noyau
    base = int(args.base, 0) if args.base else None
    attendu = None
    commit = None

    if args.manifeste:
        if not args.manifeste.is_file():
            print("manifeste absent : %s" % args.manifeste)
            return 1
        m = json.loads(args.manifeste.read_text(encoding="utf-8"))
        noyau = noyau or Path(m["noyau"]["chemin"])
        attendu = m["noyau"].get("sha256")
        base = base if base is not None else int(str(m["noyau"].get("base", BASE_DEFAUT)), 0)
        commit = m.get("commit")

    if noyau is None:
        print("il faut --manifeste ou --noyau : une adresse sans son noyau ne")
        print("veut rien dire, et la resoudre contre un autre binaire rend une")
        print("reponse fausse avec l'apparence d'une reponse juste.")
        return 1
    if not Path(noyau).is_file():
        print("noyau absent : %s" % noyau)
        return 1
    if base is None:
        base = BASE_DEFAUT

    reel = sha256(Path(noyau))
    if attendu and reel != attendu:
        print("LE NOYAU NE CORRESPOND PAS AU MANIFESTE.")
        print("  attendu : %s" % attendu)
        print("  trouve  : %s" % reel)
        print("  fichier : %s" % noyau)
        if not args.sans_verification:
            print()
            print("Refus de symboliser : entre deux constructions, l'editeur de liens")
            print("deplace tout. Une fonction innocente prendrait la place de la")
            print("coupable, et l'enquete partirait dans la mauvaise direction.")
            print("Forcer avec --sans-verification si vous savez ce que vous faites.")
            return 1

    print("noyau   : %s" % noyau)
    print("sha256  : %s" % reel)
    if commit:
        print("commit  : %s" % commit)
    print("base    : %#x" % base)
    print()

    offsets = []
    for brut in args.adresses:
        adresse = int(brut, 0)
        offset = adresse - base if adresse >= base else adresse
        offsets.append(offset)

    table = symbolise(Path(noyau), offsets)
    for brut, offset in zip(args.adresses, offsets):
        fonction, position = table.get(offset, ("<inconnu>", "??:0"))
        print("%s  ->  +%#x" % (brut, offset))
        print("    %s" % fonction)
        print("    %s" % position)
    return 0


if __name__ == "__main__":
    sys.exit(main())
