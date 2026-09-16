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


# ===========================================================================
# BOUCHAUD_SYMBOLES_SANS_OUTIL_EXTERNE_V1
# ===========================================================================
#
# Le 16 septembre, cet outil a rendu ceci sur la machine de l'utilisateur :
#
#     0x8001392340  ->  +0x1392340
#         <inconnu>
#         ??:0
#
# Il n'avait pas echoue a trouver le symbole : il n'avait AUCUN outil pour
# chercher. `addr2line` et `nm` viennent des binutils ou de LLVM, et ni l'un
# ni l'autre n'est sur le PATH d'un poste Windows ordinaire. L'outil a
# silencieusement degrade en « inconnu », ce qui ressemble a « cette adresse
# n'a pas de symbole » -- une reponse, et fausse.
#
# Une adresse qu'on ne peut pas resoudre bloque une enquete entiere. Le
# lecteur ci-dessous lit la table des symboles directement dans l'ELF, en
# Python pur : il repond partout, sans rien installer.


def symboles_elf(chemin: Path):
    """(adresse, taille, nom) de chaque fonction de `.symtab`, triees.

    Assez d'ELF pour lire une table de symboles, et rien de plus : en-tete,
    table des sections, `.symtab` et sa `.strtab` associee.
    """
    donnees = Path(chemin).read_bytes()
    if len(donnees) < 64 or donnees[:4] != b"\x7fELF":
        return []
    if donnees[4] != 2:  # ELFCLASS64
        return []
    petit = donnees[5] == 1
    ordre = "little" if petit else "big"

    def entier(debut, taille):
        return int.from_bytes(donnees[debut:debut + taille], ordre)

    e_shoff = entier(0x28, 8)
    e_shentsize = entier(0x3A, 2)
    e_shnum = entier(0x3C, 2)
    if e_shoff == 0 or e_shnum == 0:
        return []

    sections = []
    for i in range(e_shnum):
        base = e_shoff + i * e_shentsize
        if base + e_shentsize > len(donnees):
            return []
        sections.append(dict(
            type=entier(base + 0x04, 4),
            offset=entier(base + 0x18, 8),
            taille=entier(base + 0x20, 8),
            lien=entier(base + 0x28, 4),
            entsize=entier(base + 0x38, 8),
        ))

    SHT_SYMTAB, SHT_DYNSYM = 2, 11
    STT_FUNC = 2
    trouves = []
    for section in sections:
        if section["type"] not in (SHT_SYMTAB, SHT_DYNSYM):
            continue
        if section["entsize"] != 24 or section["lien"] >= len(sections):
            continue
        chaines = sections[section["lien"]]
        deb_ch, fin_ch = chaines["offset"], chaines["offset"] + chaines["taille"]
        nb = section["taille"] // 24
        for j in range(nb):
            base = section["offset"] + j * 24
            nom_off = entier(base, 4)
            info = donnees[base + 4]
            valeur = entier(base + 8, 8)
            taille = entier(base + 16, 8)
            if (info & 0xF) != STT_FUNC or valeur == 0:
                continue
            debut = deb_ch + nom_off
            if debut >= fin_ch:
                continue
            fin = donnees.find(b"\0", debut, fin_ch)
            if fin < 0:
                continue
            nom = donnees[debut:fin].decode("utf-8", errors="replace")
            if nom:
                trouves.append((valeur, taille, nom))
    trouves.sort()
    return trouves


def demangle(nom: str) -> str:
    """Rend un nom Rust lisible, au mieux et sans dependance.

    `addr2line -C` demangle ; notre lecteur de secours, non -- il rendait
    `_RNvNtNtCsjLR5ObZ1vZ6_11bouchaud_os2fs11persistance5monte`, ce qui est
    juste mais illisible, donc inutilisable dans une enquete.

    Le mangling v0 encode chaque composant par sa LONGUEUR suivie de son nom.
    Les extraire et les joindre par `::` rend le chemin. Ce n'est pas un
    demangleur complet -- generiques et durees de vie sont ignores -- et c'est
    assez pour nommer une fonction.
    """
    if not nom.startswith("_R"):
        return nom
    composants = []
    i = 2
    while i < len(nom):
        # `Cs<empreinte>_` identifie la CAISSE. Son empreinte est en base 62
        # et contient des chiffres : commencer a les lire comme une longueur
        # decoupait n'importe ou et produisait des prefixes parasites du genre
        # `ObZ1v::_11bou::`. On saute jusqu'au `_` qui la termine.
        if nom.startswith("Cs", i):
            fin = nom.find("_", i + 2)
            i = (fin + 1) if fin >= 0 else (i + 2)
            continue
        if not nom[i].isdigit():
            i += 1
            continue
        j = i
        while j < len(nom) and nom[j].isdigit():
            j += 1
        try:
            longueur = int(nom[i:j])
        except ValueError:
            i = j
            continue
        morceau = nom[j:j + longueur]
        i = j + longueur
        # Les identifiants de caisse portent une empreinte (`Cs...`) qui ne
        # dit rien a personne ; les composants vides non plus.
        if morceau and not morceau.startswith("Cs"):
            composants.append(morceau)
    if not composants:
        return nom
    return "::".join(composants)


def cherche_symbole(symboles, offset):
    """Le symbole qui CONTIENT l'offset, ou None.

    Une recherche qui rend « le dernier symbole avant » designe n'importe
    quoi quand l'adresse tombe dans un trou -- du bourrage, une section sans
    symbole. La taille du symbole est dans l'ELF : on s'en sert.
    """
    import bisect
    rang = bisect.bisect_right([s[0] for s in symboles], offset) - 1
    if rang < 0:
        return None
    adresse, taille, nom = symboles[rang]
    if taille and offset >= adresse + taille:
        return None
    return ("%s+0x%x" % (demangle(nom), offset - adresse), "??:0")


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
        # DERNIER RECOURS, ET LE SEUL QUI MARCHE PARTOUT.
        #
        # Ni addr2line ni nm sur cette machine : c'est le cas d'un poste
        # Windows sans binutils, et c'est celui qui a rendu « <inconnu> » le
        # 16 septembre. On lit l'ELF nous-memes.
        symboles = symboles_elf(noyau)
        if not symboles:
            return resultats
        for offset in offsets:
            if offset in resultats:
                continue
            trouve = cherche_symbole(symboles, offset)
            if trouve:
                resultats[offset] = trouve
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
    manquants = 0
    for brut, offset in zip(args.adresses, offsets):
        fonction, position = table.get(offset, ("<inconnu>", "??:0"))
        print("%s  ->  +%#x" % (brut, offset))
        print("    %s" % fonction)
        print("    %s" % position)
        if fonction == "<inconnu>":
            manquants += 1

    # « INCONNU » NE DOIT PAS POUVOIR VOULOIR DIRE DEUX CHOSES.
    #
    # Le 16 septembre il voulait dire « je n'ai pas d'outil pour chercher ».
    # Il ne peut plus : la table des symboles est lue ici meme. S'il reste,
    # c'est que l'adresse tombe vraiment hors de toute fonction -- et on le
    # DIT, au lieu de laisser croire a une fonction sans nom.
    if manquants:
        print()
        symboles = symboles_elf(Path(noyau))
        if not symboles:
            print("Aucun symbole lisible dans cet ELF : il est probablement")
            print("depouille. Symboliser demande le binaire NON strip, celui")
            print("que la construction ecrit a cote de l'image.")
        else:
            print("%d adresse(s) sans symbole, sur %d symboles de fonction lus"
                  % (manquants, len(symboles)))
            print("dans l'ELF. L'adresse tombe donc hors de toute fonction :")
            print("bourrage entre sections, table de saut, ou code genere sans")
            print("entree de symbole. Les adresses voisines aideront plus que")
            print("celle-ci.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
