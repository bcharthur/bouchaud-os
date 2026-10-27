#!/usr/bin/env python3
"""Une page privee chargee a la demande est complete AVANT que sa PTE ne soit
presente (P13_PAGE_REMPLIE_AVANT_PUBLICATION_V1, C5).

Regles :

  1. `AddressSpace::map_new_frame_accounted` recopie le contenu dans la frame,
     puis barriere, puis seulement `map_accounted` ; la frame vient de
     `alloc_frame_a_recouvrir` (recouverte en entier) ;
  2. le repli `PromesseBacking::File` de `faute_memoire.rs` publie sa page par
     `map_new_frame_accounted(page, &page_data, ...)`, et n'appelle plus ni
     `map_alloc_accounted` ni `space.write(page, ...)` : une PTE presente sur
     une frame vierge est executable par un fil frere sans faute ;
  3. aucun chemin de faute (`faute_memoire.rs`, `faute_cluster.rs`) n'ecrit le
     contenu d'une page apres l'avoir mappee (`space.write(` absent).

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "vm": RACINE / "src/kernel/memory/virtual.rs",
    "faute": RACINE / "src/kernel/process/thread/faute_memoire.rs",
    "cluster": RACINE / "src/kernel/process/thread/faute_cluster.rs",
}
COPIE = "core::ptr::copy_nonoverlapping(\n                contenu.as_ptr(),"
BARRIERE = "core::sync::atomic::fence(core::sync::atomic::Ordering::Release);"
PUBLIE = "self.map_accounted(virt, frame, flags, kind)"
APPEL = "mm.space.map_new_frame_accounted(\n                page,\n                &page_data,"


def corps(source: str, signature: str) -> str:
    debut = source.find(signature)
    if debut < 0:
        return ""
    ouverture = source.find("{", source.find(")", debut))
    profondeur = 0
    for i in range(ouverture, len(source)):
        if source[i] == "{":
            profondeur += 1
        elif source[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouverture:i + 1]
    return ""


def verifie(t: dict) -> list:
    fautes = []
    f = corps(t["vm"], "pub fn map_new_frame_accounted(")
    a, c, b, p = f.find("alloc_frame_a_recouvrir()"), f.find(COPIE), f.find(BARRIERE), f.find(PUBLIE)
    if not 0 <= a < c < b < p:
        fautes.append("map_new_frame_accounted : la PTE n'est pas publiee apres la copie et la barriere")
    if "contenu.len() != PAGE_SIZE as usize" not in f:
        fautes.append("map_new_frame_accounted : une page partielle laisserait des octets non ecrits")
    faute = t["faute"]
    debut = faute.find("PromesseBacking::File { .. } => {")
    repli = faute[debut:faute.find("pub fn demand_fault_stats()", debut)] if debut >= 0 else ""
    if APPEL not in repli:
        fautes.append("faute_memoire : le repli fichier ne publie pas une page deja remplie")
    if "map_alloc_accounted(" in repli:
        fautes.append("faute_memoire : le repli fichier mappe une frame vierge")
    for cle in ("faute", "cluster"):
        if "space.write(" in t[cle]:
            fautes.append(f"{cle} : contenu ecrit dans une page deja publiee")
    return fautes


def negatifs(t: dict) -> list:
    mutations = [
        ("barriere retiree", "vm", BARRIERE, ""),
        ("page partielle acceptee", "vm", "contenu.len() != PAGE_SIZE as usize", "contenu.len() > PAGE_SIZE as usize"),
        ("repli historique", "faute", APPEL, "mm.space.map_alloc_accounted(\n                page,\n                &page_data,"),
        ("ecriture apres publication", "faute", "                return FaultOutcome::IoError;\n            }\n            note.map(",
         "                return FaultOutcome::IoError;\n            }\n            mm.space.write(page, &page_data);\n            note.map("),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if avant not in t[cle]:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        m = dict(t)
        m[cle] = m[cle].replace(avant, apres, 1)
        if not verifie(m):
            echecs.append(f"mutation non detectee : {nom}")
    # La publication deplacee AVANT la copie (l'ordre historique).
    f = corps(t["vm"], "pub fn map_new_frame_accounted(")
    g = f.replace("\n        " + PUBLIE, "").replace(
        "        unsafe {", "        self.map_accounted(virt, frame, flags, kind);\n        unsafe {", 1) + ""
    g = g[:-1].rstrip() + "\n        true\n    }"
    m = dict(t)
    m["vm"] = t["vm"].replace(f, g, 1)
    if f == g or not verifie(m):
        echecs.append("mutation non detectee : publication avant copie")
    return echecs


def main() -> int:
    t = {cle: p.read_text(encoding="utf-8") for cle, p in F.items()}
    fautes = verifie(t) + negatifs(t)
    for faute in fautes:
        print(f"ECHEC {faute}")
    if fautes:
        return 1
    print("page remplie avant publication : ok (5 mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
