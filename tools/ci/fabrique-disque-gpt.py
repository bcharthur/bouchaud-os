#!/usr/bin/env python3
"""Fabrique un disque NVMe portant une vraie table GPT Bouchaud.

# Pourquoi un ecrivain INDEPENDANT

Le noyau sait ecrire une table GPT, et un test qui ferait ecrire puis relire par
le meme code ne prouverait qu'une chose : que le code est d'accord avec
lui-meme. Les fautes que ce format punit -- l'ordre mixte des octets d'un GUID,
la somme de controle calculee sur une en-tete dont le champ de somme doit etre
mis a zero, l'entete de secours a la fin du disque -- survivent toutes a un
aller-retour dans le meme code.

Ce programme ecrit la table sans partager une ligne avec le noyau. Ce que le
noyau relit ensuite est donc une table produite par quelqu'un d'autre, ce qui
est exactement la situation d'un disque partitionne par un outil tiers.

# La geometrie

    LBA 0            MBR de protection
    LBA 1            en-tete GPT primaire
    LBA 2..33        table des entrees (128 entrees de 128 octets)
    LBA 34..fin-34   zone utilisable, ou vit la partition Bouchaud
    fin-33..fin-1    copie de la table
    fin              en-tete GPT de secours
"""

import argparse
import binascii
import struct
import sys
from pathlib import Path

TAILLE_BLOC = 512
TAILLE_ENTETE = 92
TAILLE_ENTREE = 128
ENTREES = 128

# `gpt::TYPE_SYSTEME_BOUCHAUD`. Les trois premiers champs sont PETIT boutistes,
# les huit derniers octets sont ecrits tels quels : ecrire les seize dans
# l'ordre du texte donne un GUID que rien ne reconnait.
TYPE_SYSTEME_BOUCHAUD = (0xB0DC8A5D, 0x0001, 0x4B0D,
                         bytes([0x9E, 0x21, 0x42, 0x4F, 0x55, 0x43, 0x48, 0x44]))
UNIQUE_PARTITION = (0x11111111, 0x2222, 0x3333,
                    bytes([0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB]))
UNIQUE_DISQUE = (0xCAFEBABE, 0xDEAD, 0xBEEF,
                 bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]))


def guid(d1, d2, d3, d4):
    return struct.pack("<IHH", d1, d2, d3) + d4


def mbr_protecteur(blocs):
    mbr = bytearray(TAILLE_BLOC)
    e = 446
    mbr[e] = 0x00
    mbr[e + 1:e + 4] = bytes([0x00, 0x02, 0x00])
    mbr[e + 4] = 0xEE
    mbr[e + 5:e + 8] = bytes([0xFF, 0xFF, 0xFF])
    mbr[e + 8:e + 12] = struct.pack("<I", 1)
    reste = min(blocs - 1, 0xFFFFFFFF)
    mbr[e + 12:e + 16] = struct.pack("<I", reste)
    mbr[510:512] = bytes([0x55, 0xAA])
    return bytes(mbr)


def entete(mon_lba, autre_lba, premier, dernier, tableau_lba, crc_tableau):
    """L'en-tete GPT. La somme se calcule sur une copie dont le champ de somme
    est NUL -- l'oublier donne une en-tete qu'aucun systeme ne lit."""
    e = bytearray(TAILLE_ENTETE)
    e[0:8] = b"EFI PART"
    e[8:12] = struct.pack("<I", 0x00010000)
    e[12:16] = struct.pack("<I", TAILLE_ENTETE)
    e[16:20] = struct.pack("<I", 0)          # somme, posee ensuite
    e[20:24] = struct.pack("<I", 0)
    e[24:32] = struct.pack("<Q", mon_lba)
    e[32:40] = struct.pack("<Q", autre_lba)
    e[40:48] = struct.pack("<Q", premier)
    e[48:56] = struct.pack("<Q", dernier)
    e[56:72] = guid(*UNIQUE_DISQUE)
    e[72:80] = struct.pack("<Q", tableau_lba)
    e[80:84] = struct.pack("<I", ENTREES)
    e[84:88] = struct.pack("<I", TAILLE_ENTREE)
    e[88:92] = struct.pack("<I", crc_tableau)
    somme = binascii.crc32(bytes(e)) & 0xFFFFFFFF
    e[16:20] = struct.pack("<I", somme)
    return bytes(e)


def table(premier, dernier, nom):
    """La table des entrees. Seule la premiere est occupee."""
    t = bytearray(ENTREES * TAILLE_ENTREE)
    t[0:16] = guid(*TYPE_SYSTEME_BOUCHAUD)
    t[16:32] = guid(*UNIQUE_PARTITION)
    t[32:40] = struct.pack("<Q", premier)
    t[40:48] = struct.pack("<Q", dernier)
    t[48:56] = struct.pack("<Q", 0)
    # Le nom est en UTF-16LE, trente-six unites.
    brut = nom.encode("utf-16-le")[:72]
    t[56:56 + len(brut)] = brut
    return bytes(t)


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("sortie", type=Path)
    p.add_argument("--mio", type=int, default=64,
                   help="taille du disque en mebioctets")
    p.add_argument("--nom", default="BOUCHAUD")
    args = p.parse_args()

    blocs = args.mio * 1024 * 1024 // TAILLE_BLOC
    blocs_table = (ENTREES * TAILLE_ENTREE + TAILLE_BLOC - 1) // TAILLE_BLOC
    premier_utilisable = 2 + blocs_table
    dernier_utilisable = blocs - 2 - blocs_table
    if dernier_utilisable <= premier_utilisable:
        raise SystemExit("disque trop petit pour une table GPT")

    # La partition occupe toute la zone utilisable.
    entrees = table(premier_utilisable, dernier_utilisable, args.nom)
    crc_tableau = binascii.crc32(entrees) & 0xFFFFFFFF

    tableau_primaire = 2
    tableau_secours = blocs - 1 - blocs_table

    image = bytearray(blocs * TAILLE_BLOC)
    image[0:TAILLE_BLOC] = mbr_protecteur(blocs)

    primaire = entete(1, blocs - 1, premier_utilisable, dernier_utilisable,
                      tableau_primaire, crc_tableau)
    image[TAILLE_BLOC:TAILLE_BLOC + len(primaire)] = primaire

    secours = entete(blocs - 1, 1, premier_utilisable, dernier_utilisable,
                     tableau_secours, crc_tableau)
    debut_secours = (blocs - 1) * TAILLE_BLOC
    image[debut_secours:debut_secours + len(secours)] = secours

    debut = tableau_primaire * TAILLE_BLOC
    image[debut:debut + len(entrees)] = entrees
    debut = tableau_secours * TAILLE_BLOC
    image[debut:debut + len(entrees)] = entrees

    args.sortie.parent.mkdir(parents=True, exist_ok=True)
    args.sortie.write_bytes(bytes(image))
    print("DISQUE_GPT_OK image=%s blocs=%d partition=%d..%d crc_tableau=%#010x"
          % (args.sortie, blocs, premier_utilisable, dernier_utilisable, crc_tableau))
    return 0


if __name__ == "__main__":
    sys.exit(main())
