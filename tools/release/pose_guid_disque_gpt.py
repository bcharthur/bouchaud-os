#!/usr/bin/env python3
"""BOUCHAUD_CLE_USB_TEST_V1 -- le GUID de disque GPT qui identifie la cle autorisee.

    pose_guid_disque_gpt.py IMAGE HEX32          pose le GUID (en-tetes primaire ET de secours)
    pose_guid_disque_gpt.py IMAGE --lit          ecrit le GUID actuel (HEX32), ou echoue
    pose_guid_disque_gpt.py --tire               tire un GUID au hasard (os.urandom)

HEX32 : les seize octets TELS QU'ILS SONT ECRITS SUR LE DISQUE (octets 56..72
de l'en-tete), en 32 chiffres hexadecimaux -- exactement ce que le noyau
compare (`BOUCHAUD_MEDIA_AUTORISE`, `src/drivers/api/stockage_regle.rs`).
Aucune conversion d'ordre mixte : c'est une empreinte, pas un GUID a lire.

Les deux en-tetes sont reecrits et leurs sommes recalculees (sur les
`HeaderSize` octets, champ de somme a zero) ; la table des partitions n'est
pas touchee. L'image est ensuite RELUE et verifiee : signature, sommes, GUID,
et renvoi croise MyLBA/AlternateLBA des deux en-tetes. Le moindre ecart :
code 1, et rien n'est annonce.

Ecrit sans partager une ligne avec le noyau : le banc confronte ce que l'un
ecrit a ce que l'autre lit.
"""
from __future__ import annotations

import binascii
import os
import re
import struct
import sys
from pathlib import Path

BLOC = 512


def lit_entete(f, lba: int) -> bytearray:
    f.seek(lba * BLOC)
    return bytearray(f.read(BLOC))


def valide(e: bytes, lba_attendu: int) -> str | None:
    if e[0:8] != b"EFI PART":
        return "signature"
    taille = struct.unpack_from("<I", e, 12)[0]
    if not 92 <= taille <= BLOC:
        return f"taille={taille}"
    copie = bytearray(e[:taille])
    copie[16:20] = b"\0\0\0\0"
    if binascii.crc32(bytes(copie)) & 0xFFFFFFFF != struct.unpack_from("<I", e, 16)[0]:
        return "somme"
    if struct.unpack_from("<Q", e, 24)[0] != lba_attendu:
        return "my-lba"
    return None


def resomme(e: bytearray) -> None:
    taille = struct.unpack_from("<I", e, 12)[0]
    e[16:20] = b"\0\0\0\0"
    struct.pack_into("<I", e, 16, binascii.crc32(bytes(e[:taille])) & 0xFFFFFFFF)


def entetes(f) -> tuple[bytearray, int, bytearray]:
    primaire = lit_entete(f, 1)
    raison = valide(primaire, 1)
    if raison:
        raise SystemExit(f"GPT_GUID_ECHEC en-tete-primaire={raison}")
    autre = struct.unpack_from("<Q", primaire, 32)[0]
    secours = lit_entete(f, autre)
    raison = valide(secours, autre)
    if raison:
        raise SystemExit(f"GPT_GUID_ECHEC en-tete-secours={raison} lba={autre}")
    if struct.unpack_from("<Q", secours, 32)[0] != 1:
        raise SystemExit("GPT_GUID_ECHEC renvoi-croise")
    return primaire, autre, secours


def main(argv: list[str]) -> int:
    if argv[1:] == ["--tire"]:
        while True:
            g = os.urandom(16)
            if any(g):
                print(g.hex())
                return 0
    if len(argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    image = Path(argv[1])
    if argv[2] == "--lit":
        with image.open("rb") as f:
            primaire, _, secours = entetes(f)
        if primaire[56:72] != secours[56:72]:
            print("GPT_GUID_ECHEC guid-primaire-different-du-secours", file=sys.stderr)
            return 1
        print(bytes(primaire[56:72]).hex())
        return 0
    if not re.fullmatch(r"[0-9a-fA-F]{32}", argv[2]) or int(argv[2], 16) == 0:
        print("GPT_GUID_ECHEC guid-invalide (32 chiffres hexadecimaux, non nul)", file=sys.stderr)
        return 2
    guid = bytes.fromhex(argv[2])
    with image.open("r+b") as f:
        primaire, lba_secours, secours = entetes(f)
        for e, lba in ((primaire, 1), (secours, lba_secours)):
            e[56:72] = guid
            resomme(e)
            f.seek(lba * BLOC)
            f.write(e)
    with image.open("rb") as f:
        primaire, _, secours = entetes(f)
    if bytes(primaire[56:72]) != guid or bytes(secours[56:72]) != guid:
        print("GPT_GUID_ECHEC relecture", file=sys.stderr)
        return 1
    print(f"GPT_GUID_POSE image={image.name} guid={guid.hex()}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
