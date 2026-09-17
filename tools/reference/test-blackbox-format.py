#!/usr/bin/env python3
import struct, zlib, importlib.util
from pathlib import Path
HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location("extract_blackbox", HERE/"extract-blackbox.py")
m=importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

payload=b"blackbox-test\n"
raw=bytearray(m.RECORD)
raw[:8]=m.MAGIC
struct.pack_into("<HHI",raw,8,1,2,m.HEADER)
struct.pack_into("<QQQ",raw,16,20260909123456001,42,123456789)
struct.pack_into("<II",raw,40,len(payload),zlib.crc32(payload)&0xffffffff)
struct.pack_into("<QII",raw,48,777,0,0)
raw[m.HEADER:m.HEADER+len(payload)]=payload
rec=m.parse_record(bytes(raw),7)
assert rec and rec["kind"]==2 and rec["seq"]==42 and rec["payload"]==payload
raw_valide=bytes(raw)   # garde AVANT que le test de somme ne l'abime
raw[m.HEADER]^=1
assert m.parse_record(bytes(raw),7) is None
ev=struct.pack("<QQHHIQ",9,1000,1,0,1,0x80012ff162)
rows=m.decode_flight(ev)
assert rows[0]["kind_name"]=="timer-enter" and rows[0]["arg"]==0x80012ff162
# ===========================================================================
# LE SUPPORT QUI REFUSE LES LECTURES NON ALIGNEES
# ===========================================================================
#
# L'extraction du 17 septembre a echoue sur `OSError: [Errno 22] Invalid
# argument` en plein parcours des emplacements, apres avoir pourtant lu la
# table GPT. Windows n'autorise l'acces brut a un disque que par lectures
# alignees sur la taille de secteur PHYSIQUE -- 4096 sur un support 4Kn -- et
# la partition BLACKBOX commence a un octet qui n'en est pas multiple.
#
# Ce n'etait pas un detail d'outillage : c'est l'outil qui rend la trace
# lisible, et il venait de refuser la seule archive contenant une panique
# noyau. Cette epreuve fabrique le support qui refuse, et exige que
# l'extraction aboutisse quand meme.
import io

class SupportQuiRefuseLeNonAligne(io.RawIOBase):
    ALIGN = 4096

    def __init__(self, octets):
        self.octets = octets
        self.pos = 0
        self.refus = 0

    def seek(self, n, whence=0):
        self.pos = n
        return n

    def read(self, n=-1):
        if self.pos % self.ALIGN or (n and n > 0 and n % self.ALIGN):
            self.refus += 1
            raise OSError(22, "Invalid argument")
        bout = self.octets[self.pos:self.pos + n]
        self.pos += len(bout)
        return bout

    def readable(self):
        return True


# Une partition minuscule, mais commencant a un LBA dont l'octet de depart
# n'est PAS multiple de 4096 -- c'est la toute la difficulte.
SECTEURS = 4096
PREMIER = 34                      # 34 * 512 = 17408, soit 4096*4 + 1024
disque = bytearray(SECTEURS * m.SECTOR)
disque[m.SECTOR:m.SECTOR + 8] = b"EFI PART"
struct.pack_into("<Q", disque, m.SECTOR + 72, 2)      # entrees en LBA 2
struct.pack_into("<I", disque, m.SECTOR + 80, 1)      # une entree
struct.pack_into("<I", disque, m.SECTOR + 84, 128)    # de 128 octets
entree = 2 * m.SECTOR
disque[entree:entree + 16] = b"\x01" * 16            # GUID de type non nul
struct.pack_into("<Q", disque, entree + 32, PREMIER)
struct.pack_into("<Q", disque, entree + 40, PREMIER + 15)
disque[entree + 56:entree + 56 + 2 * len(m.PART_NAME)] = m.PART_NAME.encode("utf-16-le")
disque[PREMIER * m.SECTOR:PREMIER * m.SECTOR + m.RECORD] = bytes(raw_valide)

support = SupportQuiRefuseLeNonAligne(bytes(disque))
lecteur = m.LecteurBrut(support)
assert lecteur.alignement == m.SECTOR * 8, (
    "l'alignement doit etre DECOUVERT a 4096, pas suppose a 512"
)
premier, dernier, nom = m.parse_gpt_partition(lecteur)
assert nom == m.PART_NAME and premier == PREMIER, (premier, dernier, nom)
lu = lecteur.lit(PREMIER * m.SECTOR, m.RECORD)
assert lu is not None, "la lecture non alignee doit aboutir malgre le refus"
relu = m.parse_record(lu, 0)
assert relu and relu["seq"] == 42 and relu["payload"] == payload, relu
assert support.refus >= 1, "le support doit bien avoir refuse au moins une fois"

print("BLACKBOX_FORMAT_TEST_OK")
