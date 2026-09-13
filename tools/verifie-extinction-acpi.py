#!/usr/bin/env python3
"""Garde-fou : « Eteindre » coupe le courant d'une MACHINE, pas d'un emulateur.

# Le defaut, vecu sur la machine de reference

L'utilisateur a choisi « Eteindre » dans le menu demarrer. L'ecran s'est fige,
les ventilateurs ont continue de tourner, et il a fallu tenir le bouton
d'alimentation -- ce qui emporte au passage le releve de vol qu'on venait
d'ecrire pour comprendre la session.

`shutdown()` n'ecrivait que sur des ports d'EMULATEUR : `0xF4` (isa-debug-exit),
`0x604` (QEMU), `0xB004` (Bochs), `0x4004` (VirtualBox). Aucun n'existe sur un
chipset reel. Le commentaire de tete le disait meme en toutes lettres -- « une
vraie extinction ACPI demanderait de lire les tables ACPI » -- et c'etait la
liste des choses qui manquaient, pas une justification.

# Ce que la vraie extinction demande

1. Le RSDP, puis la FADT (signature `FACP`) pour le bloc de controle PM1a.
2. Le DSDT, pour l'objet AML `\\_S5_` : ses deux premiers elements sont les
   valeurs `SLP_TYP` a ecrire. Elles ne sont PAS constantes -- 0 sur beaucoup
   de chipsets, 5 sur d'autres, 7 sur certains portables. En ecrire une en dur
   eteint une machine sur trois et endort les autres.
3. `PM1x_CNT = (SLP_TYP << 10) | SLP_EN`.

# Ce qui est verifie

Que la decouverte ait lieu au DEMARRAGE et non au moment de couper, que la
valeur `SLP_TYP` vienne du DSDT, que le bit `SLP_EN` soit pose, que l'attente
de `SCI_EN` soit bornee, et que `shutdown()` tente la voie ACPI apres avoir
essaye les conventions d'emulateur.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
S5 = RACINE / "src/kernel/acpi_s5.rs"
AML = RACINE / "src/kernel/acpi_s5/aml.rs"
POWER = RACINE / "src/kernel/power.rs"
MAIN = RACINE / "src/main.rs"


def sans_commentaires(texte):
    return "\n".join(
        l for l in texte.splitlines() if not l.lstrip().startswith("//")
    )


def main():
    fautes = []
    for chemin in (S5, AML, POWER, MAIN):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin)
            return 1

    s5 = sans_commentaires(S5.read_text(encoding="utf-8"))
    aml = sans_commentaires(AML.read_text(encoding="utf-8"))
    power = sans_commentaires(POWER.read_text(encoding="utf-8"))
    main_rs = sans_commentaires(MAIN.read_text(encoding="utf-8"))

    if "b\"FACP\"" not in s5:
        fautes.append(
            "acpi_s5.rs : la FADT n'est plus cherchee. Sans elle, aucun port "
            "PM1 : il ne reste que les ports d'emulateur, qui ne coupent rien."
        )
    prepare = s5.split("pub fn prepare(", 1)[-1].split("pub fn disponible(", 1)[0]
    if "extrait_s5(" not in prepare:
        fautes.append(
            "acpi_s5.rs : la valeur SLP_TYP ne vient plus du DSDT. Ecrite en "
            "dur, elle eteint une machine sur trois et endort les autres dans "
            "un etat dont elles ne savent pas revenir."
        )
    if "SLP_EN" not in s5:
        fautes.append(
            "acpi_s5.rs : le bit SLP_EN n'est plus pose ; c'est lui qui "
            "declenche, le reste n'est qu'un numero d'etat."
        )
    m = re.search(r"const SLP_TYP_SHIFT: u16 = (\d+);", s5)
    if m is None or int(m.group(1)) != 10:
        fautes.append(
            "acpi_s5.rs : le champ SLP_TYP n'est plus au bit 10 du registre "
            "de controle PM1 (ACPI 6.x, table des registres PM1)."
        )

    eteint = s5.split("pub fn eteint()", 1)[-1]
    if ">= limite" not in eteint or "monotonic_ns()" not in eteint:
        fautes.append(
            "acpi_s5.rs : l'attente de SCI_EN n'est plus bornee en temps. Un "
            "micrologiciel qui ne rend pas la main bloquerait l'extinction "
            "pour toujours, interruptions masquees."
        )
    if "spin_loop()" not in eteint:
        fautes.append(
            "acpi_s5.rs : l'attente de SCI_EN ne relache plus le processeur."
        )

    if "pub fn prepare(" not in s5:
        fautes.append("acpi_s5.rs : la decouverte au demarrage a disparu.")
    if "acpi_s5::prepare(" not in main_rs:
        fautes.append(
            "main.rs : l'extinction n'est plus preparee au demarrage. Chercher "
            "le RSDP, la FADT puis `_S5_` au moment de couper, c'est parcourir "
            "la memoire physique au pire moment -- et le releve de vol ne "
            "porterait plus la preuve que la machine SAIT s'eteindre."
        )

    if "acpi_s5::eteint()" not in power:
        fautes.append(
            "power.rs : `shutdown()` ne tente plus la voie ACPI. Il ne reste "
            "que les ports d'emulateur, et la machine de reference resterait "
            "allumee, ecran fige, jusqu'au bouton d'alimentation."
        )
    else:
        # APRES les conventions d'emulateur : sous QEMU celles-ci ont deja
        # coupe, et sur une vraie machine elles n'ont rien fait du tout.
        avant = power.find("QEMU_ACPI_SHUTDOWN")
        apres = power.find("acpi_s5::eteint()")
        if avant < 0 or apres < avant:
            fautes.append(
                "power.rs : la voie ACPI est tentee AVANT les conventions "
                "d'emulateur ; les scenarios QEMU perdraient leur code de "
                "sortie, donc leur verdict."
            )
    for entete, quoi in (("pub fn shutdown(", "l'extinction"), ("pub fn reboot(", "le redemarrage")):
        bloc = power.split(entete, 1)[-1].split("\npub fn ", 1)[0]
        if "vide_avant_extinction" not in bloc:
            fautes.append(
                "power.rs : le releve de vol n'est plus vide avant %s. Une "
                "machine qui s'eteint VRAIMENT emporte alors la fin de la "
                "session -- exactement ce qu'on cherche a relire." % quoi
            )

    # La lecture AML doit refuser ce qu'elle ne reconnait pas plutot que de
    # deviner une valeur SLP_TYP.
    if "_ => None," not in aml:
        fautes.append(
            "acpi_s5/aml.rs : une operande AML inconnue n'est plus refusee. "
            "Deviner une valeur SLP_TYP met la machine en veille au lieu de "
            "l'eteindre."
        )
    if "elements < 2" not in aml:
        fautes.append(
            "acpi_s5/aml.rs : un paquet `_S5_` de moins de deux elements est "
            "accepte ; la valeur PM1b serait tiree de l'objet AML voisin."
        )

    if fautes:
        print("extinction ACPI : %d probleme(s)\n" % len(fautes))
        for f in fautes:
            print("  - %s\n" % f)
        return 1
    print(
        "extinction ACPI : FADT et DSDT lus au demarrage, SLP_TYP tire de "
        "`_S5_`, SLP_EN pose au bit 13, attente de SCI_EN bornee, voie ACPI "
        "tentee apres les conventions d'emulateur, releve de vol vide avant"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
