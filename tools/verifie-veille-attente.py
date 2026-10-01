#!/usr/bin/env python3
"""La veille d'attente vive tourne sur tous les coeurs, et ne touche a rien.

BOUCHAUD_VEILLE_ATTENTE_VIVE_V1

Une tache prete qui attend son coeur doit etre vue PENDANT qu'elle attend,
avec ce qui occupe ce coeur. La sonde de blocage historique ne tourne que sur
l'IRQ du coeur zero : un coeur zero pris la rendait muette (ligne de base
Scheduler NG, SMP4 : 5,5 s d'attente sur le coeur zero, aucun releve).

Le garde exige :
  1. l'appel depuis le tic du BSP (`timer.rs`) ET depuis l'IPI de quantum des
     AP (`reschedule.rs`) ;
  2. un seul passage a la fois, borne dans le temps (CAS sur l'echeance) ;
  3. lecture seule : aucune transition d'etat, aucune publication, aucun
     verrou de registre (la table est lue par `tasks()`, sure en IRQ) ;
  4. des rapports bornes (VEILLE_RAPPORTS_MAX) et un resume dans `smpstat`.

Fail-closed ; trois tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
VEILLE = "src/kernel/process/thread/veille_attente.rs"
TIMER = "src/arch/x86_64/idt/timer.rs"
IPI = "src/arch/x86_64/idt/reschedule.rs"
METRIQUES = "src/kernel/process/thread/metriques.rs"
FICHIERS = (VEILLE, TIMER, IPI, METRIQUES)


def sans_commentaires(t: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in t.splitlines())


def verifie(racine: Path) -> list[str]:
    try:
        src = {f: sans_commentaires((racine / f).read_text(encoding="utf-8")) for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []
    for f in (TIMER, IPI):
        if "crate::kernel::task::veille_attentes_vives();" not in src[f]:
            fautes.append(f"{f} : la veille n'est plus appelee depuis ce tic")
    v = src[VEILLE]
    if "VEILLE_PROCHAINE_NS\n        .compare_exchange(" not in v:
        fautes.append(f"{VEILLE} : la veille n'est plus bornee par un CAS sur son echeance")
    for interdit in (".endort(", ".reveille(", ".meurt(", "tue_parquee", "publish_ready", "registre_tache(", "condamne("):
        if interdit in v:
            fautes.append(f"{VEILLE} : la veille modifie l'ordonnancement ({interdit})")
    if "VEILLE_RAPPORTS_MAX" not in v or "[SCHED-NG-VEILLE]" not in src[METRIQUES]:
        fautes.append("rapports non bornes ou resume absent de smpstat")
    return fautes


def mutation(fichier: str, a: str, b: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp)
        for f in FICHIERS:
            d = copie / f
            d.parent.mkdir(parents=True, exist_ok=True)
            d.write_text((RACINE / f).read_text(encoding="utf-8"), encoding="utf-8")
        c = copie / fichier
        t = c.read_text(encoding="utf-8")
        if a not in t:
            return False
        c.write_text(t.replace(a, b, 1), encoding="utf-8")
        return bool(verifie(copie))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("veille d'attente vive : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (IPI, "        crate::kernel::task::veille_attentes_vives();\n", ""),
        (VEILLE, "        VEILLE_PIRE_NS.fetch_max(attente, Ordering::Relaxed);",
         "        VEILLE_PIRE_NS.fetch_max(attente, Ordering::Relaxed);\n        publish_ready(index);"),
        (VEILLE, "    if VEILLE_PROCHAINE_NS\n        .compare_exchange(", "    if false && VEILLE_PROCHAINE_NS\n        .compare_exchange_("),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"veille d'attente vive : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"VEILLE_ATTENTE_VIVE_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
