#!/usr/bin/env python3
"""Prouve structurellement les invariants de l'ordonnanceur sans verrou global.

Le gros verrou (`smp_lock`) a ete supprime (BOUCHAUD_BKL_SUPPRIME_V1) : les
clauses qui verifiaient son contrat de domaine et son budget sont parties avec
lui, et `verifie-bkl-supprime.py` garde son absence. Restent ici les
mecanismes qui l'ont remplace dans l'ordonnanceur.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent

FICHIERS_ORDONNANCEUR = [
    "src/arch/x86_64/idt/timer.rs",
    "src/arch/x86_64/idt/reschedule.rs",
    "src/kernel/process/thread/commutation.rs",
    "src/kernel/process/thread/ordonnancement.rs",
    "src/kernel/process/thread/preemption.rs",
]

ACQUISITION = re.compile(r"smp_lock::(?:enter|try_enter|try_enter_depuis_zero)\s*\(")


def lit(relatif: str) -> str:
    return (RACINE / relatif).read_text(encoding="utf-8")


def main() -> int:
    fautes: list[str] = []

    for relatif in FICHIERS_ORDONNANCEUR:
        source = lit(relatif)
        if ACQUISITION.search(source):
            fautes.append(f"{relatif}: acquisition BKL restante")

    modeles = lit("src/kernel/process/thread/modeles.rs")
    commutation = lit("src/kernel/process/thread/commutation.rs")
    courant = lit("src/kernel/process/thread/courant.rs")
    ordonnanceur = lit("src/kernel/process/thread/ordonnancement.rs")
    timer = lit("src/arch/x86_64/idt/timer.rs")
    sommeil = lit("src/kernel/process/thread/sommeil.rs")
    echeances = lit("src/kernel/scheduler/echeances.rs")
    blocage = lit("src/kernel/process/thread/blocage.rs")
    lifecycle = lit("src/kernel/process/thread/lifecycle.rs")
    debut_sortie = ordonnanceur.find("fn commute_sortie_definitive_si_possible(")
    fin_sortie = ordonnanceur.find("\n/// Rend la main", debut_sortie)
    sortie_definitive = ordonnanceur[debut_sortie:fin_sortie]
    ordre_sortie = [
        sortie_definitive.find("complete_switch_handoff();"),
        sortie_definitive.find("commence_transition_ordonnanceur()"),
        sortie_definitive.find("wake_sleepers();"),
        sortie_definitive.find("pick_next(cur, cpu_id)"),
        sortie_definitive.find("switch_to(cur, next);"),
        sortie_definitive.find("termine_transition_ordonnanceur();"),
    ]

    preuves = {
        "CAS generique des champs de tache": "pub fn compare_exchange(" in modeles,
        "revendication on_cpu -1 -> cpu": "fn revendique_candidate(" in ordonnanceur
            and ".compare_exchange(-1, cpu as i8)" in ordonnanceur,
        "porte de transition per-CPU": "TRANSITION_ORDONNANCEUR" in courant,
        "ordonnanceur sans aucun point d'accroche du verrou global":
            "smp_lock" not in ordonnanceur and "pub fn schedule() -> bool {" in ordonnanceur,
        "timer bottom-half sans BKL": "flush_interface_irq()" in timer,
        "alarmes protegees par verrou classe": "LockClass::SchedulerAlarms" in sommeil
            and "static mut ALARMS" not in sommeil,
        "balayage echeances revendique par CAS": "commence_balayage(now)" in sommeil
            and "compare_exchange(" in echeances
            and "fetch_min(minimum_ns" in echeances,
        "reveils arbitres Blocked vers Ready":
            "state.echange(TaskState::Blocked, TaskState::Ready)" in sommeil
            and "state.echange(TaskState::Blocked, TaskState::Ready)" in blocage
            and ".echange(TaskState::Blocked, TaskState::Ready)" in lifecycle,
        # QUATRE SITES, ET LE COMPTE EXACT EST LA REGLE.
        #
        # Trois jusqu'ici : le coeur secondaire, la boucle d'attente du BSP, et
        # la reprise apres cette boucle. Un quatrieme a ete ajoute : la sortie
        # immediate d'une tache qui se termine SANS lancement synchrone en
        # cours -- un travailleur de fond n'a personne a faire revenir, et
        # attendre que toutes les taches soient zombie le faisait tourner sans
        # fin des qu'un service perpetuel existait.
        #
        # Le compte reste EXACT et non « au moins » : c'est ce qui oblige a
        # relire cette regle quand une sortie est ajoutee, au lieu de laisser
        # un site s'installer sans que personne ne verifie qu'il passe bien par
        # la porte locale.
        "sorties definitives sous porte locale":
            lifecycle.count("commute_sortie_definitive_si_possible(") == 4
            and "switch_to(" not in lifecycle
            and debut_sortie >= 0
            and fin_sortie > debut_sortie
            and all(position >= 0 for position in ordre_sortie)
            and ordre_sortie == sorted(ordre_sortie),
    }
    for preuve, presente in preuves.items():
        if not presente:
            fautes.append(f"preuve absente: {preuve}")

    if fautes:
        print("ordonnanceur sans BKL : ECHEC")
        for faute in fautes:
            print(f"  - {faute}")
        return 1

    print(
        "ok  ordonnanceur: 0 acquisition BKL, revendication CAS, transition "
        "per-CPU, alarmes classees et bottom-half IRQ sans verrou global"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
