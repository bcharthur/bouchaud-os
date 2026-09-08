#!/usr/bin/env python3
import struct, sys
from pathlib import Path
ET_DYN, PT_LOAD = 3, 1
def fail(m): print(f"ERREUR: {m}", file=sys.stderr); raise SystemExit(1)
# CE FICHIER PORTE DEUX ROLES, ET C'EST CE QUI LE RENDAIT ROUGE
#
# Il est appele par `build-reference-stage2.ps1` avec le chemin de l'ELF qu'on
# vient de construire -- c'est un OUTIL. Et il s'appelle `verifie-*.py`, donc
# la barriere d'architecture le decouvre et le lance SANS ARGUMENT -- c'est
# alors une regle. Sans argument, il rendait « usage: ... » et un code non nul,
# donc la barriere etait rouge en permanence, donc plus personne ne la
# regardait -- ce qui coute plus cher que la regle ne rapporte.
#
# Sans argument, il cherche donc l'ELF a l'endroit conventionnel. S'il n'y est
# pas -- une machine qui n'a pas construit le noyau UEFI, ce qui est le cas
# normal en integration continue rapide --, il le DIT et passe. Ce qu'une
# barriere ne peut pas verifier, elle doit le dire, pas le simuler.
CONVENTIONNELS = (
    "target/x86_64-bouchaud_os_uefi/debug/bouchaud-os",
    "target/x86_64-bouchaud_os_uefi/debug/bouchaud-os.exe",
    "target/x86_64-bouchaud_os_uefi/release/bouchaud-os",
)
if len(sys.argv) > 2: fail("usage: verifie-reference-uefi-elf.py [kernel-elf]")
if len(sys.argv) == 2:
    p = Path(sys.argv[1])
    if not p.is_file(): fail(f"ELF introuvable: {p}")
else:
    racine = Path(__file__).resolve().parents[2]
    trouve = [racine / c for c in CONVENTIONNELS if (racine / c).is_file()]
    if not trouve:
        print("uefi-elf : aucun noyau UEFI construit, verification non executee")
        raise SystemExit(0)
    p = trouve[0]
d = p.read_bytes()
if len(d) < 64 or d[:4] != b"\x7fELF" or d[4] != 2 or d[5] != 1: fail("ELF 64-bit little-endian invalide")
if struct.unpack_from("<H", d, 16)[0] != ET_DYN: fail("noyau UEFI non ET_DYN")
phoff = struct.unpack_from("<Q", d, 32)[0]
entsz = struct.unpack_from("<H", d, 54)[0]
phnum = struct.unpack_from("<H", d, 56)[0]
loads = []
for i in range(phnum):
    off = phoff + i * entsz
    if off + 56 > len(d): fail("program headers tronques")
    p_type, _, _, vaddr, _, _, memsz, _ = struct.unpack_from("<IIQQQQQQ", d, off)
    if p_type == PT_LOAD and memsz: loads.append((vaddr, vaddr + memsz))
if not loads: fail("aucun PT_LOAD")
print(f"UEFI_ELF_PATH={p.resolve()}")
print("UEFI_ELF_TYPE=ET_DYN")
print(f"UEFI_PT_LOAD_MIN=0x{min(x[0] for x in loads):x}")
print(f"UEFI_PT_LOAD_MAX_EXCLUSIVE=0x{max(x[1] for x in loads):x}")
print("BOUCHAUD_REFERENCE_UEFI_PIE_OK")
