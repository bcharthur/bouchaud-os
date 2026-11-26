#!/usr/bin/env python3
"""BOUCHAUD_VDSO_V1 -- le vDSO embarque est celui de ses sources, et il est sur.

Le noyau embarque `src/arch/x86_64/vdso/vdso.so` (include_bytes!) pour que sa
construction ne depende d'aucun compilateur C. Ce garde verifie, sans outil
externe (lecture ELF en Python) :

1. l'empreinte des sources (`vdso.so.sources`) est celle de vdso.c et vdso.lds :
   une source modifiee sans `construit.sh` ne passe pas ;
2. l'image est un ELF64 x86-64 partage, UN seul segment PT_LOAD, lisible et
   executable mais JAMAIS inscriptible (page commune a tous les processus) ;
3. AUCUNE relocation (rien ne la reloge : une relocation serait un pointeur faux) ;
4. les symboles que cherchent la glibc et musl sont exportes ;
5. elle tient en une page (la projection noyau la suppose petite) ;
6. le noyau : vvar jamais executable ni inscriptible, vDSO jamais inscriptible,
   `mprotect(PROT_WRITE)` refuse sur `Backing::Vdso`, et les ecritures pour le
   compte d'un processus (CHILD_SETTID, clear_child_tid, copies) passent par
   `write_utilisateur`.

Fail-closed ; `--negatif` rejoue des mutations qui DOIVENT etre refusees.
"""
import hashlib
import struct
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
VDSO = RACINE / "src/arch/x86_64/vdso"
SYMBOLES = ("__vdso_clock_gettime", "__vdso_gettimeofday", "__vdso_time", "__vdso_getcpu")


def lit_elf(donnees: bytes) -> dict:
    if donnees[:4] != b"\x7fELF" or donnees[4] != 2 or donnees[5] != 1:
        raise ValueError("pas un ELF64 petit-boutiste")
    (e_type, e_machine) = struct.unpack_from("<HH", donnees, 16)
    (e_phoff, e_shoff) = struct.unpack_from("<QQ", donnees, 32)
    (e_phentsize, e_phnum, e_shentsize, e_shnum, e_shstrndx) = struct.unpack_from("<HHHHH", donnees, 54)
    segments = []
    for i in range(e_phnum):
        p_type, p_flags = struct.unpack_from("<II", donnees, e_phoff + i * e_phentsize)
        segments.append((p_type, p_flags))
    sections = []
    for i in range(e_shnum):
        sh_name, sh_type = struct.unpack_from("<II", donnees, e_shoff + i * e_shentsize)
        sh_offset, sh_size = struct.unpack_from("<QQ", donnees, e_shoff + i * e_shentsize + 24)
        sh_link, = struct.unpack_from("<I", donnees, e_shoff + i * e_shentsize + 40)
        sh_entsize, = struct.unpack_from("<Q", donnees, e_shoff + i * e_shentsize + 56)
        sections.append(dict(type=sh_type, offset=sh_offset, size=sh_size, link=sh_link, entsize=sh_entsize))
    noms = set()
    for s in sections:
        if s["type"] == 11:  # SHT_DYNSYM
            chaines = sections[s["link"]]
            for j in range(s["size"] // 24):
                st_name, = struct.unpack_from("<I", donnees, s["offset"] + j * 24)
                debut = chaines["offset"] + st_name
                fin = donnees.index(b"\0", debut)
                noms.add(donnees[debut:fin].decode())
    return dict(type=e_type, machine=e_machine, segments=segments, sections=sections, symboles=noms, taille=len(donnees))


def verifie_image(elf: dict) -> list:
    fautes = []
    if elf["type"] != 3 or elf["machine"] != 62:
        fautes.append("vdso.so : pas un objet partage x86-64")
    charges = [f for t, f in elf["segments"] if t == 1]
    if len(charges) != 1:
        fautes.append(f"vdso.so : {len(charges)} segments PT_LOAD (1 attendu)")
    for f in charges:
        if f & 2:
            fautes.append("vdso.so : segment PT_LOAD INSCRIPTIBLE (page commune a tous les processus)")
        if not f & 1 or not f & 4:
            fautes.append("vdso.so : segment PT_LOAD non lisible ou non executable")
    if any(s["type"] in (4, 9) for s in elf["sections"]):
        fautes.append("vdso.so : relocations presentes")
    for nom in SYMBOLES:
        if nom not in elf["symboles"]:
            fautes.append(f"vdso.so : symbole {nom} absent")
    if elf["taille"] > 4096:
        fautes.append(f"vdso.so : {elf['taille']} octets (une page attendue)")
    return fautes


def verifie_sources() -> list:
    attendu = {}
    for ligne in (VDSO / "vdso.so.sources").read_text().splitlines():
        empreinte, nom = ligne.split()
        attendu[nom] = empreinte
    fautes = []
    for nom in ("vdso.c", "vdso.lds"):
        reel = hashlib.sha256((VDSO / nom).read_bytes()).hexdigest()
        if attendu.get(nom) != reel:
            fautes.append(f"{nom} modifie sans reconstruire vdso.so (src/arch/x86_64/vdso/construit.sh)")
    return fautes


def verifie_noyau(t: dict) -> list:
    fautes = []
    v = t["vdso"]
    if "let lecture = vmm::PTE_PRESENT | vmm::PTE_USER | vmm::PTE_NO_EXEC;" not in v:
        fautes.append("vdso.rs : la page vvar doit etre lecture seule et jamais executable")
    if "let execution = vmm::PTE_PRESENT | vmm::PTE_USER;" not in v or "PTE_WRITE" in v.split("fn projette")[1].split("\n}\n")[0]:
        fautes.append("vdso.rs : le vDSO ne doit jamais etre projete inscriptible")
    if "Backing::Vdso { mapping_start, .. }" not in t["mem"]:
        fautes.append("mem.rs : mprotect(PROT_WRITE) n'est plus refuse sur le vDSO")
    if "prot & PROT_EXEC != 0 && mapping_start == crate::kernel::vdso::adresse_vvar()" not in t["mem"]:
        fautes.append("mem.rs : mprotect(PROT_EXEC) n'est plus refuse sur vvar")
    if "effective.drapeaux & !crate::kernel::vmm::PTE_WRITE" not in t["faute"]:
        fautes.append("faute_memoire.rs : une faute sur le vDSO pourrait le projeter inscriptible")
    for fichier, motif in (
        ("proc", "space.write_utilisateur(demande.child_tid"),
        ("cycle", "space.write_utilisateur(clear,"),
        ("usercopy", "space.write_utilisateur(addr, bytes)"),
        ("compat", "space.write_utilisateur(addr, data)"),
    ):
        if motif not in t[fichier]:
            fautes.append(f"{fichier} : ecriture pour le compte d'un processus hors write_utilisateur")
    return fautes


def textes() -> dict:
    chemins = {
        "vdso": "src/kernel/memory/vdso.rs",
        "mem": "src/compat/linux/mem.rs",
        "faute": "src/kernel/process/thread/faute_memoire.rs",
        "proc": "src/compat/linux/proc.rs",
        "cycle": "src/kernel/process/thread/lifecycle.rs",
        "usercopy": "src/kernel/native/abi/usercopy.rs",
        "compat": "src/compat/linux/mod.rs",
    }
    return {k: (RACINE / v).read_text(encoding="utf-8") for k, v in chemins.items()}


def negatifs(image: bytes, t: dict) -> list:
    """Chaque mutation doit etre refusee ; rend celles qui passent."""
    passent = []
    elf = lit_elf(image)
    mute = dict(elf, segments=[(1, 7)])
    if not verifie_image(mute):
        passent.append("segment inscriptible")
    mute = dict(elf, sections=elf["sections"] + [dict(type=4, offset=0, size=0, link=0, entsize=24)])
    if not verifie_image(mute):
        passent.append("relocation")
    mute = dict(elf, symboles=elf["symboles"] - {"__vdso_clock_gettime"})
    if not verifie_image(mute):
        passent.append("symbole manquant")
    m = dict(t, proc=t["proc"].replace("space.write_utilisateur(demande.child_tid", "space.write(demande.child_tid"))
    if not verifie_noyau(m):
        passent.append("CHILD_SETTID non verifie")
    m = dict(t, mem=t["mem"].replace("Backing::Vdso { mapping_start, .. }", "Backing::Zero"))
    if not verifie_noyau(m):
        passent.append("mprotect inscriptible")
    return passent


def main() -> int:
    image = (VDSO / "vdso.so").read_bytes()
    t = textes()
    fautes = verifie_sources() + verifie_image(lit_elf(image)) + verifie_noyau(t)
    passent = negatifs(image, t)
    fautes += [f"mutation non detectee : {p}" for p in passent]
    if fautes:
        print("vDSO : regle violee")
        for f in fautes:
            print("  " + f)
        return 1
    print("vDSO : ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
