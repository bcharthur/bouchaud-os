#!/usr/bin/env python3
"""Garde-fou : QEMU doit pouvoir prendre la FORME de la machine de reference.

# Le defaut, mesure le 16 septembre 2026

`run-reference-stage2.ps1` tournait en `-smp 1`, sans xHCI et sans NVMe. Sur
cette forme, le journal dit :

    SMP4_AP_STARTED count=0 reason=single-vcpu
    BOUCHAUD_HWPROBE_XHCI absent
    BOUCHAUD_NVME_ABSENT
    BOUCHAUD_STAGE2_ENTREE_DECIDEE claviers_usb=0 ps2_clavier=1

Autrement dit, aucun des trois chemins qui posent probleme sur la machine :

  * la frontiere SMP, celle qui a double-faute le 14 septembre -- sans second
    coeur, la garde d'amorcage ne tourne meme pas ;
  * le chemin USB HID, celui du clavier muet -- sans xHCI, le systeme tombe
    sur le PS/2 ;
  * le disque.

QEMU validait donc une image qui ne partageait presque rien avec ce qui allait
tourner sur la TRIGKEY.

# Ce qui a ete verifie une fois la forme donnee

    SMP4_AP_STARTED count=15 expected=15
    SMP_HANDOFF_BEFORE_STI ... cpus_en_ligne=16
    SMP_HANDOFF_AFTER_FIRST_IRQ vector=0x20 rsp=0x18000014d50
    BOUCHAUD_STAGE2_ENTREE_DECIDEE claviers_usb=1 souris_usb=1 ps2_clavier=0
    BOUCHAUD_NVME_GREEN bdf=00:02.0 blocs=524288
    [USB-HID-POINT] genre=clavier evenements=32  apres seize frappes injectees

Le meme `rsp` que le releve physique, et un clavier USB qui repond. C'est ce
dernier point qui a retire le chemin HID generique de la liste des suspects
pour le clavier de la machine.

# Ce qui est verifie ici

1. Le profil existe et donne seize coeurs.
2. Il branche un xHCI AVEC un clavier et une souris, sinon le chemin HID
   reste mort et le systeme retombe sur le PS/2.
3. Il branche un NVMe.
4. Il DIT ce qu'il ne reproduit pas : une forme proche n'est pas la machine.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
RUNNER = RACINE / "tools/reference/run-reference-stage2.ps1"


def main():
    if not RUNNER.exists():
        print("  - fichier absent : %s" % RUNNER)
        return 1
    texte = RUNNER.read_text(encoding="utf-8")
    fautes = []

    if "$CommeTrigkey" not in texte:
        fautes.append(
            "run-reference-stage2.ps1 : le profil `-CommeTrigkey` a disparu. "
            "Sans lui, QEMU tourne a un seul coeur, sans xHCI et sans NVMe : "
            "ni la frontiere SMP, ni le chemin USB HID, ni le disque ne sont "
            "exerces, et l'image validee ne partage presque rien avec celle qui "
            "tourne sur la machine."
        )
        print("forme trigkey : 1 probleme(s)")
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    if not re.search(r"\$Coeurs\s*=\s*16", texte):
        fautes.append(
            "run-reference-stage2.ps1 : le profil ne donne plus seize coeurs. "
            "La machine de reference en a seize, et c'est le nombre qui fait "
            "tourner la garde d'amorcage SMP -- a un seul coeur elle n'existe "
            "pas, et `SMP_HANDOFF_BEFORE_STI` ne sort jamais."
        )

    for motif, pourquoi in (
        (r"qemu-xhci",
         "sans controleur xHCI le systeme retombe sur le PS/2 "
         "(`BOUCHAUD_STAGE2_ENTREE_DECIDEE ps2_clavier=1`) et tout le chemin "
         "du clavier muet reste mort"),
        (r"usb-kbd",
         "un xHCI sans clavier ne prouve rien : c'est `evenements=` du point "
         "de terminaison clavier qui doit pouvoir monter"),
        (r"usb-mouse",
         "la souris est le TEMOIN : sur la machine, elle marche pendant que "
         "le clavier se tait, et c'est ce contraste qui oriente l'enquete"),
        (r"-device\", \"nvme",
         "sans NVMe, `BOUCHAUD_NVME_ABSENT` et le disque n'est jamais teste"),
    ):
        if not re.search(motif, texte):
            fautes.append(
                "run-reference-stage2.ps1 : le profil ne branche plus `%s` -- %s."
                % (motif.replace("\\", ""), pourquoi)
            )

    # Une forme proche n'est pas la machine, et le script doit le dire.
    if "ne reproduit PAS" not in texte:
        fautes.append(
            "run-reference-stage2.ps1 : le profil ne dit plus ce qu'il ne "
            "reproduit pas. Processeur emule, xHCI de QEMU et non l'AMD de la "
            "TRIGKEY, clavier USB simple et non un recepteur Logitech unifie : "
            "taire ces ecarts ferait prendre un QEMU vert pour une preuve "
            "physique, ce qui est exactement l'erreur que le reste de ce depot "
            "s'emploie a rendre impossible."
        )

    if fautes:
        print("forme trigkey : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "forme trigkey : seize coeurs, xHCI avec clavier et souris, NVMe, et "
        "les ecarts avec la vraie machine annonces"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
