#!/usr/bin/env python3
"""Garde-fou : une porte IDT absente ne doit pas se lire « DOUBLE FAULT ».

# Le defaut, lu sur la machine de reference

Releve physique du 14 septembre 2026. La derniere ligne normale est
`SMP_BOOT_GUARD_EXIT`, la suivante est une double faute :

    vector = 8 DOUBLE FAULT
    code   = 0
    CR2    = 0
    task   = <aucune>
    « exception initiale <non entree dans un gestionnaire> »

Aucune information sur le vecteur reellement demande.

L'IDT ne peuplait que quinze vecteurs ; les 241 autres sortaient de
`InterruptDescriptorTable::new()`, donc avec le bit Present a zero. Le
processeur, face a une porte absente, leve #NP (vecteur 11) avec un code
d'erreur qui NOMME la porte manquante -- mais `segment_not_present` n'etait pas
installe non plus. La porte du #NP etant elle-meme absente, la seconde faute
pendant la livraison de la premiere donne une #DF a code zero, sans CR2 et sans
gestionnaire logiciel. C'est exactement la signature observee.

Trois vecteurs non peuplés etaient atteignables a cet instant precis :

    0x27  parasite du PIC maitre  -- banal sur un vrai PC, quasi absent de QEMU
    0x2F  parasite du PIC esclave -- portait le gestionnaire ATA secondaire,
          qui acquittait inconditionnellement
    0xFF  parasite du LAPIC       -- `enable_local_apic` programme ce vecteur
          dans le SVR, et rien ne l'installait

# Ce qui est verifie

1. #NP est installe et decode son code d'erreur.
2. Les trois parasites ont chacun leur gestionnaire.
3. Les vecteurs SMP existants n'ont pas ete perdus.
4. Les parasites du 8259 CONSULTENT l'ISR au lieu d'acquitter a l'aveugle.
5. La frontiere SMP est photographiee avant la restauration de l'IF.
6. L'ecran de faute n'accuse plus une pile d'amorcage d'etre corrompue.
7. IRQ0 et l'IPI de replanification restent SANS TACHE tant que le scheduler
   n'est pas active.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
IDT = RACINE / "src/arch/x86_64/idt.rs"
IMPREVUS = RACINE / "src/arch/x86_64/idt/imprevus.rs"
POLITIQUE = RACINE / "src/arch/x86_64/idt/politique_vecteurs.rs"
SMP = RACINE / "src/arch/x86_64/smp.rs"
ECRAN = RACINE / "src/platform/pc/ecran_faute.rs"
TIMER = RACINE / "src/arch/x86_64/idt/timer.rs"
RESCHEDULE = RACINE / "src/arch/x86_64/idt/reschedule.rs"


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
    fautes = []
    for chemin in (IDT, IMPREVUS, POLITIQUE, SMP, ECRAN, TIMER, RESCHEDULE):
        if not chemin.exists():
            print("  - fichier absent : %s" % chemin)
            return 1

    idt = IDT.read_text(encoding="utf-8")
    imprevus = IMPREVUS.read_text(encoding="utf-8")
    politique = POLITIQUE.read_text(encoding="utf-8")
    smp = SMP.read_text(encoding="utf-8")
    ecran = ECRAN.read_text(encoding="utf-8")

    # --- 1. #NP installe et decode -----------------------------------------
    installe = corps(imprevus, "fn installe_vecteurs_imprevus(")
    if installe is None:
        fautes.append(
            "imprevus.rs : l'installation des vecteurs imprevus a disparu. Une "
            "porte IDT absente redeviendrait une double faute muette."
        )
    else:
        if "segment_not_present" not in installe:
            fautes.append(
                "imprevus.rs : #NP n'est plus installe. C'est le SEUL mecanisme "
                "du processeur qui nomme la porte manquante ; sans lui, la "
                "faute se lit « DOUBLE FAULT » et rien d'autre."
            )
        for constante, nom in (
            ("VECTEUR_PIC_PARASITE_MAITRE", "0x27 (parasite PIC maitre)"),
            ("VECTEUR_PIC_PARASITE_ESCLAVE", "0x2F (parasite PIC esclave)"),
            ("VECTEUR_LAPIC_PARASITE", "0xFF (parasite LAPIC)"),
        ):
            if constante not in installe:
                fautes.append(
                    "imprevus.rs : le vecteur %s n'est plus installe. Un x86 "
                    "physique le produit tot ou tard, et une porte absente y "
                    "est fatale." % nom
                )

    if "installe_vecteurs_imprevus(" not in idt:
        fautes.append(
            "idt.rs : les vecteurs imprevus ne sont plus installes dans l'IDT."
        )

    np = corps(imprevus, "fn segment_not_present_handler(")
    if np is None:
        fautes.append("imprevus.rs : le gestionnaire #NP a disparu.")
    else:
        if "vecteur_idt_absent(" not in np:
            fautes.append(
                "imprevus.rs : le gestionnaire #NP ne decode plus son code "
                "d'erreur ; il perdrait le numero de vecteur, seule "
                "information que cette exception apporte."
            )
        if "BOUCHAUD_IDT_NOT_PRESENT" not in np:
            fautes.append(
                "imprevus.rs : le gestionnaire #NP ne publie plus le vecteur "
                "absent."
            )
        # EARLY-BOOT SAFE : ce chemin s'execute avant toute tache.
        for interdit, raison in (
            ("smp_lock::enter", "il prendrait le gros verrou"),
            ("alloc::", "il allouerait"),
            ("Vec::", "il allouerait"),
            ("format!", "il allouerait pour formater"),
        ):
            if interdit in np:
                fautes.append(
                    "imprevus.rs : le gestionnaire #NP n'est plus early-boot "
                    "safe (%s). Il s'execute avant qu'une tache existe." % raison
                )

    # --- 2. Les vecteurs SMP n'ont pas ete perdus ---------------------------
    for vecteur in ("RESCHEDULE_VECTOR", "PANIC_STOP_VECTOR", "TLB_SHOOTDOWN_VECTOR"):
        if "IDT[smp::%s as usize]" % vecteur not in idt:
            fautes.append(
                "idt.rs : le vecteur SMP %s n'est plus installe." % vecteur
            )

    # --- 3. Les parasites 8259 consultent l'ISR -----------------------------
    #
    # LA REGLE QUI COMPTE. Acquitter un parasite acquitte l'interruption de
    # quelqu'un d'autre : c'est precisement ce que faisait le gestionnaire ATA
    # secondaire installe sur 0x2F.
    for fonction, decision in (
        ("fn pic_parasite_maitre_handler(", "fin_irq7("),
        ("fn pic_parasite_esclave_handler(", "fin_irq15("),
    ):
        bloc = corps(imprevus, fonction)
        if bloc is None:
            fautes.append("imprevus.rs : %s a disparu." % fonction)
            continue
        if decision not in bloc or "lit_isr(" not in bloc:
            fautes.append(
                "imprevus.rs : %s acquitte sans consulter l'ISR du 8259. Un "
                "parasite acquitterait alors l'interruption reellement en "
                "service." % fonction
            )

    lapic = corps(imprevus, "fn lapic_parasite_handler(")
    if lapic is None:
        fautes.append("imprevus.rs : le gestionnaire de parasite LAPIC a disparu.")
    elif "eoi" in lapic.lower():
        fautes.append(
            "imprevus.rs : le parasite LAPIC envoie un EOI. Intel SDM Vol.3 "
            "11.9 : une interruption parasite ne met rien en service et n'en "
            "attend aucun."
        )

    # --- 4. Les decisions restent pures et verifiables ----------------------
    for interdit in ("use crate::", "unsafe ", "asm!"):
        if interdit in politique:
            fautes.append(
                "politique_vecteurs.rs : le module de decision n'est plus pur "
                "(%s). Il ne serait plus verifiable sur l'hote." % interdit.strip()
            )

    # --- 5. La frontiere SMP est photographiee ------------------------------
    drop = corps(smp, "impl Drop for SmpBootstrapGuard")
    if drop is None:
        fautes.append("smp.rs : le garde d'amorcage SMP a disparu.")
    else:
        if "photographie_avant_sti()" not in drop:
            fautes.append(
                "smp.rs : l'etat n'est plus photographie avant la restauration "
                "de l'IF. La premiere interruption apres la frontiere "
                "redeviendrait une inconnue."
            )
        rang_photo = drop.find("photographie_avant_sti()")
        rang_sti = drop.find("interrupts::enable()")
        if rang_photo >= 0 and rang_sti >= 0 and rang_photo > rang_sti:
            fautes.append(
                "smp.rs : la photographie est prise APRES le `sti`. Elle ne "
                "decrirait plus l'etat dans lequel la premiere interruption "
                "est livree."
            )
    if "SMP_HANDOFF_BEFORE_STI" not in smp or "SMP_HANDOFF_AFTER_FIRST_IRQ" not in smp:
        fautes.append("smp.rs : les marqueurs de frontiere SMP ont disparu.")
    premiere = corps(smp, "pub fn note_premiere_irq(")
    if premiere is None:
        fautes.append("smp.rs : le releve de la premiere interruption a disparu.")
    elif "compare_exchange" not in premiere:
        fautes.append(
            "smp.rs : le releve de la premiere interruption n'est plus "
            "one-shot ; il tracerait a chaque tick."
        )

    # --- 6. L'ecran de faute n'accuse plus une pile d'amorcage --------------
    if "HORS TAS NOYAU" in ecran:
        fautes.append(
            "ecran_faute.rs : l'ecran accuse de nouveau une pile d'etre hors "
            "du tas. La pile d'amorcage est allouee par le chargeur et n'a "
            "jamais eu a vivre dans le tas : ce libelle envoie chercher la "
            "panne au mauvais endroit."
        )
    if "premiere_porte_absente()" not in ecran:
        fautes.append(
            "ecran_faute.rs : l'ecran n'affiche plus la porte IDT absente "
            "capturee avant la double faute."
        )

    # --- 7. IRQ0 reste une horloge tant que le scheduler dort ---------------
    #
    # LA REGLE LA PLUS IMPORTANTE DE CE FICHIER.
    #
    # `bootstrap_in_progress` tombe dans `Drop for SmpBootstrapGuard`, juste
    # AVANT le `sti`, alors que `scheduler_enabled` est encore faux et
    # qu'aucune tache n'est installee. Un handler qui ne teste que le premier
    # drapeau entre donc dans le chemin complet -- reveils, watchdog,
    # comptabilite de tache, preemption -- dans un etat ou `CURRENT` n'existe
    # pas. C'est la frontiere exacte ou le releve du 14 septembre place sa
    # double faute, `task=<aucune>`.
    for chemin, nom, entete in (
        (TIMER, "timer.rs", "fn timer_interrupt_handler("),
        (RESCHEDULE, "reschedule.rs", "fn reschedule_interrupt_handler("),
    ):
        source = chemin.read_text(encoding="utf-8")
        bloc = corps(source, entete)
        if bloc is None:
            fautes.append("%s : %s a disparu." % (nom, entete))
            continue
        if "timer_runtime_pret(" not in bloc:
            fautes.append(
                "%s : la frontiere pre-scheduler a disparu. Le handler "
                "redeviendrait dependant du seul `bootstrap_in_progress()`, "
                "qui tombe AVANT l'activation du scheduler et AVANT qu'une "
                "tache existe." % nom
            )
            continue
        # Le seul test toléré sur `bootstrap_in_progress` est celui qui passe
        # par la decision commune : un test nu reintroduirait le defaut.
        nu = re.search(
            r"if\s+!?\s*smp::bootstrap_in_progress\(\)\s*\{", bloc
        )
        if nu is not None:
            fautes.append(
                "%s : un test nu sur `bootstrap_in_progress()` est revenu. La "
                "decision doit passer par `timer_runtime_pret`, qui consulte "
                "AUSSI `scheduler_enabled`." % nom
            )
        # La barriere doit preceder tout ce qui suppose une tache.
        rang = bloc.find("timer_runtime_pret(")
        for apres, quoi in (
            ("note_rip_timer", "le releve de RIP par tache"),
            ("flush_interface_irq", "le vidage des reveils"),
            ("watchdog_from_timer", "le watchdog"),
            ("echantillonne_tache_bsp", "l'echantillonnage de tache"),
            ("stall_ipi_observe", "l'observation de blocage"),
        ):
            position = bloc.find(apres)
            if position >= 0 and position < rang:
                fautes.append(
                    "%s : %s s'execute AVANT la frontiere pre-scheduler. Il "
                    "suppose une tache courante, qui n'existe pas encore."
                    % (nom, quoi)
                )

    politique_pure = POLITIQUE.read_text(encoding="utf-8")
    if "fn timer_runtime_pret(" not in politique_pure:
        fautes.append(
            "politique_vecteurs.rs : la decision du timer n'est plus une "
            "fonction pure ; elle cesserait d'etre verifiable sur l'hote."
        )

    if fautes:
        print("vecteurs imprevus : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "vecteurs imprevus : #NP decode, parasites PIC/LAPIC servis selon l'ISR, "
        "frontiere SMP photographiee, ecran de faute honnete, IRQ0 sans tache "
        "avant le scheduler"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
