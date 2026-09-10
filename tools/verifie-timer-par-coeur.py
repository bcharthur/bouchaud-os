#!/usr/bin/env python3
"""Garde-fou : chaque coeur en ligne a son propre battement.

# Le defaut, mesure sur la machine de reference

Seul le mode TSC-deadline etait implemente. Or TSC-deadline est une
fonctionnalite Intel (`CPUID.1:ECX[24]`), absente du Ryzen 7 5800H du TRIGKEY
(`BOUCHAUD_HWPROBE_CPU vendor=AuthenticAMD logical=16`).

Aucun coeur n'avait donc de timer local, et le seul battement du systeme
restait le PIT -- une source unique, routee vers un seul coeur. L'enregistreur
de vol physique ne contient QUE des evenements de `cpu=0`, et les compteurs de
tick le confirment : `timer0` avance de 1 978 a 13 500, `timer1`, `timer2` et
`timer3` restent a zero.

Quinze coeurs sur seize etaient en ligne, comptes dans `SMP4_AP_STARTED
count=15`, et incapables de preempter quoi que ce soit.

# Le second defaut, revele par le correctif du premier

Un AP attend `SCHEDULER_ENABLED` en `hlt` : il dort, il ne scrute pas. Il
n'arme SON timer qu'apres etre sorti de cette attente. Sans interruption pour
l'en sortir, il n'arme rien ; et sans timer arme, aucune interruption ne vient.

Le coup de pouce initial n'etait envoye QUE si aucun timer local n'existait.
Le rendre inconditionnel etait la seule facon d'amorcer.

# Ce qui est verifie

Que le repli periodique existe, que le mode decide AVANT toute ecriture de MSR
-- ecrire `IA32_TSC_DEADLINE` sur un processeur sans TSC-deadline leve une
faute de protection generale --, que le coup de pouce initial soit
inconditionnel, et que le battement soit MESURE et non deduit du compte de
coeurs en ligne.
"""

import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
SMP = RACINE / "src/arch/x86_64/smp.rs"


def sans_commentaires(texte):
    return "\n".join(
        ligne for ligne in texte.splitlines()
        if not ligne.lstrip().startswith("//")
    )


def corps(source, entete):
    debut = source.find(entete)
    if debut < 0:
        return None
    i = source.find("{", debut)
    if i < 0:
        return None
    profondeur = 0
    for j in range(i, len(source)):
        if source[j] == "{":
            profondeur += 1
        elif source[j] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[i:j + 1]
    return None


def main():
    if not SMP.exists():
        print("  - fichier absent : src/arch/x86_64/smp.rs")
        return 1
    smp = sans_commentaires(SMP.read_text(encoding="utf-8"))
    fautes = []

    init = corps(smp, "fn init_local_scheduler_timer()")
    if init is None:
        fautes.append("smp.rs : la mise en service du timer local a disparu.")
    else:
        if "LVT_PERIODIQUE" not in init:
            fautes.append(
                "smp.rs : le repli periodique a disparu. Sans lui, un processeur "
                "sans TSC-deadline -- tout AMD -- n'a aucun timer local, et un "
                "seul coeur bat."
            )
        if "calibre_timer_local()" not in init:
            fautes.append(
                "smp.rs : le compte du timer periodique n'est plus calibre ; il "
                "serait devine, et le quantum n'aurait plus de duree connue."
            )

    arme = corps(smp, "pub fn arm_local_scheduler_timer()")
    if arme is None:
        fautes.append("smp.rs : le rearmement du timer local a disparu.")
    else:
        if "MODE_TIMER_LOCAL" not in arme:
            fautes.append(
                "smp.rs : le rearmement ne consulte plus le MODE. Ecrire "
                "`IA32_TSC_DEADLINE` sur un processeur qui ne l'a pas leve une "
                "faute de protection generale : le mode doit decider AVANT "
                "l'ecriture, et non un simple drapeau « un timer est actif »."
            )

    enable = corps(smp, "pub fn enable_scheduler()")
    if enable is None:
        fautes.append("smp.rs : enable_scheduler a disparu.")
    else:
        if "broadcast_reschedule();" not in enable:
            fautes.append(
                "smp.rs : le coup de pouce initial a disparu ; les AP resteraient "
                "en hlt sans jamais armer leur timer."
            )
        # Le rendre conditionnel reintroduit exactement le verrou d'amorcage.
        if "if !local_scheduler_timer_enabled()" in enable:
            fautes.append(
                "smp.rs : le coup de pouce initial est redevenu conditionnel. "
                "Un AP endormi n'arme pas son timer, et un AP sans timer ne se "
                "reveille pas : c'est le verrou d'amorcage que cette regle "
                "existe pour empecher."
            )
        if "mesure_battement_par_coeur" not in enable:
            fautes.append(
                "smp.rs : le battement n'est plus mesure a l'amorcage. "
                "Programmer un timer ne prouve pas qu'il se declenche, et "
                "« en ligne » n'a jamais voulu dire « bat »."
            )

    mesure = corps(smp, "pub fn mesure_battement_par_coeur(")
    if mesure is None:
        fautes.append("smp.rs : la mesure du battement a disparu.")
    else:
        if "quantums_recus" not in mesure:
            fautes.append(
                "smp.rs : la mesure ne lit plus le compteur d'interruptions de "
                "quantum ; elle deduirait le battement au lieu de l'observer."
            )
        if "BOUCHAUD_SMP_BATTEMENT" not in mesure:
            fautes.append(
                "smp.rs : la mesure ne publie plus son resultat par coeur ; un "
                "coeur muet redeviendrait invisible."
            )

    if fautes:
        print("timer par coeur : %d probleme(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print(
        "timer par coeur : repli periodique calibre, mode consulte avant tout "
        "MSR, coup de pouce inconditionnel, battement mesure par coeur"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
