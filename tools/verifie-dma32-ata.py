#!/usr/bin/env python3
"""Le bus-master IDE ne recoit que de la memoire DMA sous 4 Gio.

BOUCHAUD_DMA32_V1

# Le defaut

L'arene DMA est taillee en haut de la plus grande region RAM. Avec 8 Gio
(smoke Ladybird, `-m 8192`), cette region est au-dessus de 4 Gio : `alloc_dma`
rendait des adresses de 36 bits, et le bus-master IDE, qui n'en a que 32,
desactivait la lecture DMA (`adresses-hors-contraintes`). Tous les autres
lanceurs donnant 4 Gio, le defaut n'apparaissait que sous Ladybird -- et le
smoke #371 a tourne entierement en PIO sans qu'aucun controle ne le dise.

# La regle

  * dans le module `dma` de `ata.rs`, aucune allocation par `alloc_dma(` :
    seulement `alloc_dma32(` (et `free_dma32(` pour rendre) ;
  * la verification des quatre contraintes (`contraintes_violees`) reste
    appelee avant d'armer le DMA ;
  * `physical.rs` definit `alloc_dma32` et retire la reserve 32 bits des
    frames utilisateur (`reserve32`).

Fail-closed : un fichier ou une fonction introuvable est une faute. Deux tests
negatifs.
"""

import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
ATA = "src/drivers/block/ata.rs"
PHYS = "src/kernel/memory/physical.rs"


def sans_commentaires(texte: str) -> str:
    return "\n".join(re.sub(r"//.*", "", ligne) for ligne in texte.splitlines())


def module_dma(texte: str) -> str | None:
    debut = texte.find("mod dma {")
    if debut < 0:
        return None
    profondeur = 0
    for i in range(texte.find("{", debut), len(texte)):
        if texte[i] == "{":
            profondeur += 1
        elif texte[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return texte[debut : i + 1]
    return None


def verifie(racine: Path) -> list[str]:
    fautes = []
    try:
        ata = sans_commentaires((racine / ATA).read_text(encoding="utf-8"))
        phys = sans_commentaires((racine / PHYS).read_text(encoding="utf-8"))
    except OSError as e:
        return [f"lecture impossible : {e}"]
    dma = module_dma(ata)
    if dma is None:
        return [f"{ATA} : module `dma` introuvable"]
    if re.search(r"\balloc_dma\s*\(", dma):
        fautes.append(f"{ATA} : le module dma alloue par alloc_dma (adresses possiblement > 4 Gio)")
    if not re.search(r"\balloc_dma32\s*\(", dma):
        fautes.append(f"{ATA} : le module dma n'alloue plus par alloc_dma32")
    if not re.search(r"\bcontraintes_violees\s*\(\s*prdt\s*,\s*tampon\s*\)", dma):
        fautes.append(f"{ATA} : la verification des quatre contraintes n'est plus appelee")
    if "pub fn alloc_dma32(" not in phys:
        fautes.append(f"{PHYS} : alloc_dma32 introuvable")
    if "reserve32" not in phys:
        fautes.append(f"{PHYS} : la reserve 32 bits n'est plus retiree des frames")
    return fautes


def mutation(fichier: str, avant: str, apres: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp)
        for f in (ATA, PHYS):
            dest = copie / f
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text((RACINE / f).read_text(encoding="utf-8"), encoding="utf-8")
        cible = copie / fichier
        texte = cible.read_text(encoding="utf-8")
        if avant not in texte:
            return False
        cible.write_text(texte.replace(avant, apres, 1), encoding="utf-8")
        return bool(verifie(copie))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("DMA 32 bits de l'ATA : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (ATA, "crate::kernel::memory::alloc_dma32(TAMPON)", "crate::kernel::memory::alloc_dma(TAMPON)"),
        (ATA, "let fautes = contraintes_violees(prdt, tampon);", "let fautes = 0u8;"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"DMA 32 bits de l'ATA : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"DMA32_ATA_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
