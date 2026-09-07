#!/usr/bin/env python3
import struct, sys
from pathlib import Path
ET_DYN, PT_LOAD = 3, 1
def fail(m): print(f"ERREUR: {m}", file=sys.stderr); raise SystemExit(1)
if len(sys.argv) != 2: fail("usage: verifie-reference-uefi-elf.py <kernel-elf>")
p = Path(sys.argv[1])
if not p.is_file(): fail(f"ELF introuvable: {p}")
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
