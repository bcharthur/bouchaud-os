#!/usr/bin/env python3
"""Fabrique un disque GPT portant une partition BOUCHAUD-BLACKBOX.

Sert de CIBLE a l'enregistreur de vol sous QEMU. Sans une telle partition,
`blackbox_storage_ready()` est faux et tout le chemin d'ecriture sort
immediatement : le defaut du 16 septembre -- l'archive qui s'arrete a 7,6 s
d'une session de vingt minutes -- n'existait donc que sur la machine, ce qui
est la pire facon d'exister.

    python3 tools/reference/fabrique-disque-blackbox.py disque.img --mio 64
"""
import argparse, struct, uuid, zlib
from pathlib import Path

SECTEUR = 512
NOM = "BOUCHAUD-BLACKBOX"
# Type « Microsoft basic data » : le noyau cherche le NOM, pas le type.
TYPE_DONNEES = uuid.UUID("EBD0A0A2-B9E5-4433-87C0-68B6B72699C7")


def guid_mixte(u: uuid.UUID) -> bytes:
    """GUID au format mixte-endian du GPT."""
    champs = u.fields
    return (struct.pack("<IHH", champs[0], champs[1], champs[2])
            + struct.pack(">HHI", champs[3] << 8 | champs[4], 0, 0)[:2]
            + u.bytes[8:])


def entete(lba_courant, lba_secours, premier, dernier, lba_table, entrees, crc_table, guid_disque):
    hdr = bytearray(92)
    hdr[0:8] = b"EFI PART"
    struct.pack_into("<IIII", hdr, 8, 0x00010000, 92, 0, 0)
    struct.pack_into("<QQQQ", hdr, 24, lba_courant, lba_secours, premier, dernier)
    hdr[56:72] = guid_mixte(guid_disque)
    struct.pack_into("<QIII", hdr, 72, lba_table, entrees, 128, crc_table)
    struct.pack_into("<I", hdr, 16, zlib.crc32(bytes(hdr)) & 0xFFFFFFFF)
    return bytes(hdr) + b"\0" * (SECTEUR - 92)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("sortie")
    ap.add_argument("--mio", type=int, default=64)
    ap.add_argument("--partition-mio", type=int, default=32)
    a = ap.parse_args()

    secteurs = a.mio * 1024 * 1024 // SECTEUR
    entrees = 128
    secteurs_table = entrees * 128 // SECTEUR          # 32
    premier_utilisable = 2 + secteurs_table            # 34
    dernier_utilisable = secteurs - 1 - secteurs_table - 1

    part_secteurs = a.partition_mio * 1024 * 1024 // SECTEUR
    part_premier = premier_utilisable
    part_dernier = part_premier + part_secteurs - 1
    if part_dernier > dernier_utilisable:
        raise SystemExit("partition plus grande que le disque")

    table = bytearray(entrees * 128)
    table[0:16] = guid_mixte(TYPE_DONNEES)
    table[16:32] = guid_mixte(uuid.uuid4())
    struct.pack_into("<QQQ", table, 32, part_premier, part_dernier, 0)
    nom = NOM.encode("utf-16-le")
    table[56:56 + len(nom)] = nom
    crc_table = zlib.crc32(bytes(table)) & 0xFFFFFFFF
    guid_disque = uuid.uuid4()

    disque = bytearray(secteurs * SECTEUR)
    # MBR protecteur.
    disque[0x1BE] = 0x00
    disque[0x1BE + 4] = 0xEE
    struct.pack_into("<II", disque, 0x1BE + 8, 1, min(secteurs - 1, 0xFFFFFFFF))
    disque[0x1FE:0x200] = b"\x55\xaa"

    disque[SECTEUR:2 * SECTEUR] = entete(
        1, secteurs - 1, premier_utilisable, dernier_utilisable, 2, entrees,
        crc_table, guid_disque)
    disque[2 * SECTEUR:2 * SECTEUR + len(table)] = table

    secours_table = secteurs - 1 - secteurs_table
    disque[secours_table * SECTEUR:secours_table * SECTEUR + len(table)] = table
    disque[(secteurs - 1) * SECTEUR:secteurs * SECTEUR] = entete(
        secteurs - 1, 1, premier_utilisable, dernier_utilisable, secours_table,
        entrees, crc_table, guid_disque)

    Path(a.sortie).write_bytes(bytes(disque))
    print(f"BLACKBOX_DISQUE {a.sortie} secteurs={secteurs} "
          f"partition={NOM} first_lba={part_premier} last_lba={part_dernier} "
          f"octets={part_secteurs * SECTEUR}")


if __name__ == "__main__":
    main()
