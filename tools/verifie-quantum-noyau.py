#!/usr/bin/env python3
"""Un fil noyau qui epuise son quantum devant des taches pretes est preempte.

BOUCHAUD_QUANTUM_NOYAU_V1

Un fil noyau n'etait preemptable que sur DEMANDE CIBLEE (reveil d'une tache
sensible a la latence). Un fil noyau qui scrute sans dormir tenait donc son
coeur : la veille d'attente vive a mesure `usb-hid` 732 ms sur un AP pendant
scheduler-ng-banc SMP4, et `fork-exit` p99 1,26 s dans ce demarrage.

Le garde exige :
  1. `accorde_preemption_noyau` accorde aussi au quantum epuise, et SOUS LA
     MEME condition de surete (`preemption_noyau_sure`) que la demande ciblee ;
  2. le quantum est borne : au moins un quantum d'ordonnanceur, au plus 4 ;
  3. `fil_noyau_quantum_epuise` ne vise que les fils noyau, seulement si des
     taches attendent CE coeur, et mesure depuis la mise en route de la tranche
     (`slice_start_ns`) ;
  4. le compteur est publie dans `smpstat` (`log_reveil` appele).

Fail-closed ; cinq tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
PREEMPT = "src/kernel/scheduler/preempt.rs"
ORDO = "src/kernel/process/thread/ordonnancement.rs"
METRIQUES = "src/kernel/process/thread/metriques.rs"
FICHIERS = (PREEMPT, ORDO, METRIQUES)


def sans_commentaires(t: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in t.splitlines())


def corps(texte: str, nom: str) -> str:
    m = re.search(r"\bfn\s+" + re.escape(nom) + r"\s*\(", texte)
    if not m:
        return ""
    o = texte.find("{", m.end())
    p = 0
    for i in range(o, len(texte)):
        p += texte[i] == "{"
        p -= texte[i] == "}"
        if p == 0:
            return texte[o:i + 1]
    return ""


def verifie(racine: Path) -> list[str]:
    try:
        src = {f: sans_commentaires((racine / f).read_text(encoding="utf-8")) for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []

    a = corps(src[PREEMPT], "accorde_preemption_noyau")
    if "fil_noyau_quantum_epuise(QUANTUM_NOYAU_NS)" not in a:
        fautes.append(f"{PREEMPT} : accorde_preemption_noyau ne regarde plus le quantum noyau")
    sure = a.find("if preemption_noyau_sure()")
    retour_vrai = [m.start() for m in re.finditer(r"return true", a)]
    if sure < 0 or not retour_vrai or any(r < sure for r in retour_vrai):
        fautes.append(f"{PREEMPT} : une preemption de fil noyau est accordee hors de preemption_noyau_sure")
    if "PREEMPTIONS_NOYAU_QUANTUM.fetch_add" not in a:
        fautes.append(f"{PREEMPT} : les preemptions au quantum ne sont plus comptees")

    m = re.search(r"const QUANTUM_NOYAU_NS: u64 = ([0-9]+) \* smp::SCHED_QUANTUM_TICKS \* 1_000_000;", src[PREEMPT])
    if not m or not 1 <= int(m.group(1)) <= 4:
        fautes.append(f"{PREEMPT} : QUANTUM_NOYAU_NS hors de [1, 4] quanta d'ordonnanceur")

    q = corps(src[ORDO], "fil_noyau_quantum_epuise")
    if not re.search(r"if !current_is_kernel_task\(\) \{\s*return false;", q):
        fautes.append(f"{ORDO} : le quantum noyau ne se limite plus aux fils noyau")
    if not re.search(r"if ready_count_cpu\(cpu\) == 0 \{\s*return false;", q):
        fautes.append(f"{ORDO} : le quantum noyau preempte sans tache en attente sur ce coeur")
    if "slice_start_ns.charge()" not in q or ">= seuil_ns" not in q:
        fautes.append(f"{ORDO} : le quantum noyau ne se mesure plus depuis la mise en route de la tranche")

    if "preempt_noyau_quantum=" not in corps(src[PREEMPT], "log_reveil"):
        fautes.append(f"{PREEMPT} : log_reveil ne publie plus preempt_noyau_quantum")
    if "scheduler::preempt::log_reveil();" not in corps(src[METRIQUES], "log_smp_load"):
        fautes.append(f"{METRIQUES} : log_reveil n'est plus appele par smpstat")
    return fautes


def mutation(fichier: str, a: str, b: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        for f in FICHIERS:
            d = Path(tmp) / f
            d.parent.mkdir(parents=True, exist_ok=True)
            t = (RACINE / f).read_text(encoding="utf-8")
            if f == fichier:
                if a not in t:
                    return False
                t = t.replace(a, b, 1)
            d.write_text(t, encoding="utf-8")
        return bool(verifie(Path(tmp)))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("quantum noyau : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        # le quantum contourne la condition de surete
        (PREEMPT, "    if !ciblee && !quantum {\n        return false;\n    }",
         "    if quantum {\n        return true;\n    }\n    if !ciblee {\n        return false;\n    }"),
        # retour a la demande ciblee seule
        (PREEMPT, "crate::kernel::task::fil_noyau_quantum_epuise(QUANTUM_NOYAU_NS)", "false"),
        # quantum demesure
        (PREEMPT, "QUANTUM_NOYAU_NS: u64 = 2 *", "QUANTUM_NOYAU_NS: u64 = 50 *"),
        # preemption sans tache en attente
        (ORDO, "    if ready_count_cpu(cpu) == 0 {\n        return false;\n    }\n", ""),
        # compteur invisible
        (METRIQUES, "    crate::kernel::scheduler::preempt::log_reveil();\n", ""),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"quantum noyau : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"QUANTUM_NOYAU_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
