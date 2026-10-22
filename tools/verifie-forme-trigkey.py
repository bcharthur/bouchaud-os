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
COMMUN = RACINE / "tools/reference/qemu-windows-common.ps1"


def verifier_contrat(texte: str, commun: str) -> list[str]:
    """Check the transitive QEMU argv path, preserving SMP, xHCI, HID and NVMe."""
    erreurs = []
    def exige(vrai: bool, description: str) -> None:
        if not vrai:
            erreurs.append(description)

    exige("$CommeTrigkey" in texte and
          bool(re.search(r"if\s*\(\$CommeTrigkey\)\s*\{", texte)),
          "le profil explicite -CommeTrigkey a disparu")
    exige(bool(re.search(r"if\s*\(\$Coeurs\s*-eq\s*0\)\s*\{\s*\$Coeurs\s*=\s*16", texte)),
          "le profil Trigkey ne configure plus 16 coeurs")
    exige(bool(re.search(r"['\"]-smp['\"]\s*,\s*['\"]?\$Coeurs", texte)),
          "le parametre SMP n'est plus transmis a QEMU")
    exige(bool(re.search(r"\[string\]\$InputMode\s*=\s*'usb'", texte)) and
          bool(re.search(r"\[string\]\$Pointer\s*=\s*'mouse'", texte)),
          "le profil d'entree par defaut n'est plus clavier+souris USB")
    exige("qemu-windows-common.ps1" in texte and
          bool(re.search(r"\.\s*\(Join-Path\s+\$PSScriptRoot\s+'qemu-windows-common.ps1'\)", texte)),
          "la bibliotheque QEMU partagee n'est plus importee")
    exige("Get-BouchaudInputArguments -InputMode $InputMode -Pointer $Pointer" in texte,
          "le runner ne reprend plus les arguments xHCI/HID du helper")
    exige("function Get-BouchaudInputArguments" in commun and
          bool(re.search(r"if\s*\(\$InputMode\s*-eq\s*'usb'\)", commun)),
          "helper USB absent ou branche USB inactif")
    for motif, raison in (
        (r"['\"]qemu-xhci,id=xhci['\"]", "controleur xHCI absent"),
        (r"['\"]usb-kbd,bus=xhci\.0['\"]", "clavier USB absent"),
        (r"usb-\$Pointer,bus=xhci\.0", "souris USB dynamique absente"),
    ):
        exige(bool(re.search(motif, commun)), raison)
    exige("'-device','nvme,drive=nv,serial=BOUCHAUDTRIGKEY'" in texte and
          "id=nv,file=$Nvme,format=raw,if=none,snapshot=on" in texte,
          "NVMe virtuel Trigkey absent ou sans snapshot")
    exige("Get-BouchaudDisplayArguments" in texte and
          "Invoke-BouchaudQemu -QemuExe $QemuExe -Arguments $ArgsQemu" in texte,
          "arguments QEMU construits mais non transmis au lanceur")
    exige("ne reproduit PAS" in texte and "xHCI QEMU" in texte and
          "recepteur Logitech" in texte and "NVMe virtuel" in texte,
          "ecarts avec le materiel physique non documentes")
    return erreurs


def main() -> int:
    for path in (RUNNER, COMMUN):
        if not path.is_file():
            print(f"forme trigkey : fichier absent : {path}")
            return 1
    texte = RUNNER.read_text(encoding="utf-8-sig")
    commun = COMMUN.read_text(encoding="utf-8-sig")
    fautes = verifier_contrat(texte, commun)
    if fautes:
        print(f"forme trigkey : {len(fautes)} probleme(s)")
        for faute in fautes:
            print("  - " + faute)
        return 1
    print("forme trigkey : contrat 16 coeurs, USB HID xHCI, NVMe snapshot et ecarts physiques OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
