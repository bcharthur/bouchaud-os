#!/usr/bin/env python3
"""Fabrique un disque de donnees portant une zone de persistance V1 remplie.

    python3 tools/fs/fabrique-zone-persistance.py hdb.img

# Pourquoi cet outil existe

Le 16 septembre 2026, le premier demarrage QEMU avec l'image Ladybird est
mort avant le bureau :

    *** KERNEL PANIC *** cpu=0
    panicked at src/fs/ramfs.rs:198:8:
    LOCKDEP inversion cpu=0 held_rank=50 acquiring=vfs(50)

`montage::monte` tenait le garde RAMFS et appelait `depose`, qui le
reprenait. Le defaut avait des mois : il ne se voit que si la zone de
persistance EXISTE et porte des entrees.

Or `debut()` refuse la zone sur un volume de donnees trop petit. Tous les
demarrages QEMU tournaient sans second disque : `persistance: disque trop
petit, zone absente`, et la branche fautive n'etait jamais executee. Il a
fallu une image de 1351 mebioctets pour l'atteindre.

Ce disque-ci la reproduit en deux cent vingt-six mebioctets, sans rien
telecharger. C'est la difference entre une panne qu'on a vue une fois et une
panne qu'on peut rejouer.

# Ce qu'il ecrit

    secteur 0                 une archive USTAR minimale, pour que `tar`
                              trouve quelque chose a deplier
    secteur base              en-tete V1 : "BOPERSI1", puis le nombre
                              d'entrees en u32 petit-boutiste a l'offset 12
    secteur base + 1          la table : chemin sur 240 octets, puis la
                              taille en u64 petit-boutiste
    secteur base + 1025       le contenu

`base` vaut `total - SECTEURS_ZONE` : la zone occupe la FIN du disque, et le
disque doit depasser `SECTEURS_ZONE + SECTEUR_CONTENU` pour que `debut()`
l'accepte. Ces constantes sont celles de `src/fs/persistance.rs` ; si elles
bougent, ce script doit bouger avec elles.
"""

import argparse
import io
import sys
import tarfile

SECTOR_SIZE = 512
ENTREES_MAX = 2048
TAILLE_ENTREE = 256
CHEMIN_MAX = TAILLE_ENTREE - 16
SECTEURS_TABLE = ENTREES_MAX * TAILLE_ENTREE // SECTOR_SIZE
SECTEUR_CONTENU = 1 + SECTEURS_TABLE
SECTEURS_ZONE = 262144
MAGIE = b"BOPERSI1"


def fabrique(chemin, restaure, contenu, marge_secteurs):
    total = SECTEURS_ZONE + SECTEUR_CONTENU + marge_secteurs
    base = total - SECTEURS_ZONE

    archive = io.BytesIO()
    with tarfile.open(fileobj=archive, mode="w", format=tarfile.USTAR_FORMAT) as tar:
        for nom, octets in (
            ("bo-navigateur", b"\x7fELF" + b"\0" * 2048),
            ("etc/demo.txt", b"bonjour\n"),
        ):
            info = tarfile.TarInfo(nom)
            info.size = len(octets)
            info.mode = 0o755
            tar.addfile(info, io.BytesIO(octets))

    with open(chemin, "wb") as disque:
        disque.write(archive.getvalue())
        disque.truncate(total * SECTOR_SIZE)

        entete = bytearray(SECTOR_SIZE)
        entete[0:8] = MAGIE
        entete[12:16] = (1).to_bytes(4, "little")
        disque.seek(base * SECTOR_SIZE)
        disque.write(entete)

        entree = bytearray(TAILLE_ENTREE)
        nom = restaure.encode("ascii")
        if len(nom) > CHEMIN_MAX:
            raise SystemExit("chemin trop long : %d octets, %d au plus"
                             % (len(nom), CHEMIN_MAX))
        entree[0:len(nom)] = nom
        entree[CHEMIN_MAX:CHEMIN_MAX + 8] = len(contenu).to_bytes(8, "little")
        disque.seek((base + 1) * SECTOR_SIZE)
        disque.write(entree)

        disque.seek((base + SECTEUR_CONTENU) * SECTOR_SIZE)
        disque.write(contenu.ljust(SECTOR_SIZE, b"\0"))

    return total, base


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("sortie")
    parser.add_argument("--chemin", default="restaure/essai.txt",
                        help="chemin du fichier a restaurer sous /persist")
    parser.add_argument("--marge-secteurs", type=int, default=200_000,
                        help="secteurs au-dela du seuil de debut()")
    args = parser.parse_args()

    contenu = b"une ligne restauree depuis la zone V1\n"
    total, base = fabrique(args.sortie, args.chemin, contenu, args.marge_secteurs)

    print("DISQUE=%s" % args.sortie)
    print("SECTEURS=%d (%d Mio)" % (total, total * SECTOR_SIZE // 1024 // 1024))
    print("ZONE_V1_SECTEUR=%d" % base)
    print("FICHIER_RESTAURE=/persist/%s" % args.chemin)
    print()
    print("Attendu au demarrage :")
    print("  persistance: zone au format V1, migree au prochain sync")
    print("  persistance: 1 fichier(s) restaure(s) depuis le disque")
    print("et AUCUNE ligne LOCKDEP.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
