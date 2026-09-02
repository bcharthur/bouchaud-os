#!/usr/bin/env python3
"""Prouve structurellement que l'ordonnanceur ne reprend plus le BKL."""

import json
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

    domaine = lit("src/kernel/sync/domaine.rs")
    branches_migrees = re.findall(
        r"((?:Self::\w+\s*\|?\s*)+)=>\s*Contrat::Migre", domaine
    )
    migres = {nom for branche in branches_migrees for nom in re.findall(r"Self::(\w+)", branche)}
    if "Ordonnanceur" not in migres:
        fautes.append("domaine Ordonnanceur non declare Migre")

    budget = json.loads(lit("tools/ci/budgets/bkl-sites.json"))
    sites = budget["architecture"]["sites_bkl_par_domaine"]
    if "Ordonnanceur" in sites:
        fautes.append("budget BKL: Ordonnanceur doit disparaitre plutot que rester a zero")

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
        "coeur scheduler depth zero": "fn schedule_sans_bkl()" in ordonnanceur
            and "scheduler execute sous BKL" in ordonnanceur,
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
        "sorties definitives detachees avant switch":
            lifecycle.count("abandonne_bkl_avant_sortie_definitive();") == 2
            and "fn abandonne_bkl_avant_sortie_definitive()" in ordonnanceur,
        "sorties definitives sous porte locale":
            lifecycle.count("commute_sortie_definitive_si_possible(") == 3
            and "switch_to(" not in lifecycle
            and debut_sortie >= 0
            and fin_sortie > debut_sortie
            and all(position >= 0 for position in ordre_sortie)
            and ordre_sortie == sorted(ordre_sortie),
    }
    for preuve, presente in preuves.items():
        if not presente:
            fautes.append(f"preuve absente: {preuve}")

    debut = ordonnanceur.find("fn schedule_sans_bkl()")
    fin = ordonnanceur.find("\nfn switch_to(", debut)
    coeur = ordonnanceur[debut:fin]
    if "suspend_for_schedule" in coeur or "resume_after_schedule" in coeur:
        fautes.append("le coeur schedule_sans_bkl suspend/reprend encore le BKL")

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
