#!/usr/bin/env python3
"""Fabrique `hello.exe` : un PE32+ natif Bouchaud, sans Windows.

## Ce que ce fichier est

Un executable PE32+ AMD64 qui n'importe RIEN. Pas de kernel32, pas de CRT, pas
d'editeur de liens dynamique. Son point d'entree fait deux appels systeme
Bouchaud -- `write(1, ...)` puis `exit(0)` -- par l'instruction `syscall`,
avec la convention Linux/POSIX que ce noyau implemente deja.

C'est la difference que ce jalon doit rendre evidente : le FORMAT vient de
Windows, l'ABI vient de Bouchaud. Un `.exe` compile pour Windows ne marchera
pas davantage apres ce commit qu'avant, et le chargeur le dit.

## Pourquoi un generateur plutot qu'un binaire commite

Un binaire dans le depot est un fichier que personne ne peut relire. Ici le
programme fait douze instructions, et elles sont ecrites ci-dessous en clair
avec leur encodage. La fixture se regenere a l'identique -- aucun horodatage,
aucun sel -- ce qui permet a un test de comparer des octets.

Usage : tools/exec/fabrique-hello-exe.py [sortie]
"""

import struct
import sys
from pathlib import Path

MESSAGE = b"Hello from PE32+ on Bouchaud OS\n"

BASE_IMAGE = 0x0000_0001_4000_0000
ALIGNEMENT_SECTION = 0x1000
ALIGNEMENT_FICHIER = 0x200

RVA_TEXT = 0x1000
RVA_DATA = 0x2000

MACHINE_AMD64 = 0x8664
OPTIONAL_PE32PLUS = 0x20B
FILE_EXECUTABLE = 0x0002
FILE_LARGE_ADDRESS_AWARE = 0x0020
SUBSYSTEM_WINDOWS_CUI = 3

SCN_CODE = 0x0000_0020
SCN_INITIALISED = 0x0000_0040
SCN_MEM_EXECUTE = 0x2000_0000
SCN_MEM_READ = 0x4000_0000


def code_entree() -> bytes:
    """Le programme, instruction par instruction.

    Appels systeme Linux/POSIX, ceux que Bouchaud sert deja :
        write(fd=1, buf, len)   -> rax=1, rdi=1, rsi=buf, rdx=len
        exit(0)                 -> rax=60, rdi=0

    `mov rsi, imm64` porte l'adresse absolue du message. C'est ce qui rend une
    relocation DIR64 necessaire si l'image n'est pas chargee a `ImageBase` --
    et c'est exactement ce qu'on veut exercer.
    """
    adresse_message = BASE_IMAGE + RVA_DATA
    return b"".join([
        b"\x48\xc7\xc0\x01\x00\x00\x00",              # mov rax, 1   (write)
        b"\x48\xc7\xc7\x01\x00\x00\x00",              # mov rdi, 1   (stdout)
        b"\x48\xbe" + struct.pack("<Q", adresse_message),  # movabs rsi, message
        b"\x48\xc7\xc2" + struct.pack("<I", len(MESSAGE)), # mov rdx, len
        b"\x0f\x05",                                   # syscall
        b"\x48\xc7\xc0\x3c\x00\x00\x00",              # mov rax, 60  (exit)
        b"\x48\x31\xff",                               # xor rdi, rdi
        b"\x0f\x05",                                   # syscall
        b"\xf4",                                       # hlt (jamais atteint)
    ])


def offset_du_movabs(code: bytes) -> int:
    """Position de l'immediat 64 bits dans `.text`, pour la relocation."""
    marqueur = b"\x48\xbe"
    position = code.index(marqueur) + len(marqueur)
    return position


def aligne(valeur: int, sur: int) -> int:
    return (valeur + sur - 1) // sur * sur


def construis() -> bytes:
    code = code_entree()
    rva_relocation = RVA_TEXT + offset_du_movabs(code)

    # --- table de relocations : un bloc, une entree DIR64 -------------------
    page = rva_relocation & ~0xFFF
    decalage = rva_relocation & 0xFFF
    entrees = struct.pack("<H", (10 << 12) | decalage)  # DIR64
    entrees += struct.pack("<H", 0)                      # ABSOLUTE : bourrage
    bloc_reloc = struct.pack("<II", page, 8 + len(entrees)) + entrees

    donnees = MESSAGE + bloc_reloc
    rva_reloc_table = RVA_DATA + len(MESSAGE)

    sections = [
        # (nom, rva, contenu, caracteristiques)
        (b".text", RVA_TEXT, code, SCN_CODE | SCN_MEM_EXECUTE | SCN_MEM_READ),
        (b".rdata", RVA_DATA, donnees, SCN_INITIALISED | SCN_MEM_READ),
    ]

    offset_mz = 0
    taille_entetes_bruts = 0x40 + 4 + 20 + 240 + len(sections) * 40
    taille_entetes = aligne(taille_entetes_bruts, ALIGNEMENT_FICHIER)

    # --- corps des sections -------------------------------------------------
    corps = b""
    tables = []
    offset = taille_entetes
    for nom, rva, contenu, caracteristiques in sections:
        brut = aligne(len(contenu), ALIGNEMENT_FICHIER)
        tables.append((nom, rva, len(contenu), offset, brut, caracteristiques))
        corps += contenu + b"\0" * (brut - len(contenu))
        offset += brut

    taille_image = aligne(
        max(rva + len(contenu) for _, rva, contenu, _ in sections),
        ALIGNEMENT_SECTION,
    )

    # --- en-tete MZ ---------------------------------------------------------
    mz = bytearray(0x40)
    mz[0:2] = b"MZ"
    struct.pack_into("<I", mz, 0x3C, 0x40)

    # --- en-tete COFF -------------------------------------------------------
    coff = struct.pack(
        "<HHIIIHH",
        MACHINE_AMD64,
        len(sections),
        0,              # horodatage : zero, pour que la fixture soit stable
        0, 0,           # table de symboles absente
        240,            # taille de l'en-tete optionnel
        FILE_EXECUTABLE | FILE_LARGE_ADDRESS_AWARE,
    )

    # --- en-tete optionnel PE32+ -------------------------------------------
    opt = struct.pack(
        "<HBBIIIII",
        OPTIONAL_PE32PLUS,
        14, 0,                       # version de l'editeur de liens
        len(code),                   # taille du code
        len(donnees),                # donnees initialisees
        0,                           # donnees non initialisees
        RVA_TEXT,                    # point d'entree
        RVA_TEXT,                    # base du code
    )
    opt += struct.pack("<Q", BASE_IMAGE)
    opt += struct.pack("<II", ALIGNEMENT_SECTION, ALIGNEMENT_FICHIER)
    opt += struct.pack("<HHHHHH", 6, 0, 0, 0, 6, 0)   # versions OS/image/sous-systeme
    opt += struct.pack("<I", 0)                        # reserve
    opt += struct.pack("<II", taille_image, taille_entetes)
    opt += struct.pack("<I", 0)                        # somme de controle
    opt += struct.pack("<HH", SUBSYSTEM_WINDOWS_CUI, 0)
    opt += struct.pack("<QQQQ", 0x100000, 0x1000, 0x100000, 0x1000)  # pile/tas
    opt += struct.pack("<II", 0, 16)                   # drapeaux, repertoires

    repertoires = []
    for index in range(16):
        if index == 5:   # BASE RELOCATION TABLE
            repertoires.append(struct.pack("<II", rva_reloc_table, len(bloc_reloc)))
        else:
            repertoires.append(struct.pack("<II", 0, 0))
    opt += b"".join(repertoires)
    assert len(opt) == 240, len(opt)

    # --- table des sections -------------------------------------------------
    table = b""
    for nom, rva, taille_virtuelle, offset_brut, taille_brute, caracteristiques in tables:
        table += nom.ljust(8, b"\0")
        table += struct.pack(
            "<IIIIIIHHI",
            taille_virtuelle, rva, taille_brute, offset_brut,
            0, 0, 0, 0, caracteristiques,
        )

    entetes = bytes(mz) + b"PE\0\0" + coff + opt + table
    entetes += b"\0" * (taille_entetes - len(entetes))
    return entetes + corps


def main() -> int:
    sortie = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("hello.exe")
    image = construis()
    sortie.parent.mkdir(parents=True, exist_ok=True)
    sortie.write_bytes(image)
    print(f"{sortie} : {len(image)} octets, PE32+ AMD64 sans importation")
    return 0


if __name__ == "__main__":
    sys.exit(main())
