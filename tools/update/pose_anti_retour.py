#!/usr/bin/env python3
"""BOUCHAUD_ANTI_RETOUR_V1 -- pose le registre anti-retour dans META (blocs 3 et 4).

    pose_anti_retour.py IMAGE META_PREMIER_LBA PLANCHER

Encodeur INDEPENDANT de system/update-core/src/anti_retour.rs (struct +
zlib.crc32) : le banc confronte ce que l'un ecrit a ce que l'autre lit.
Registre : "BOAR0001", sequence=1, plancher, release du slot A = plancher,
release du slot B = 0, reserve 0, CRC32 des 44 premiers octets. Copie 0
seulement (bloc 3) ; le bloc 4 reste a zero.
"""
import struct
import sys
import zlib

image, lba, plancher = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
corps = b"BOAR0001" + struct.pack("<QQQQ", 1, plancher, plancher, 0) + b"\0" * 4
bloc = corps + struct.pack("<I", zlib.crc32(corps) & 0xFFFFFFFF)
with open(image, "r+b") as f:
    f.seek((lba + 3) * 512)
    f.write(bloc + b"\0" * (512 - len(bloc)))
print(f"ANTI_RETOUR_POSE plancher={plancher} lba={lba + 3}")
