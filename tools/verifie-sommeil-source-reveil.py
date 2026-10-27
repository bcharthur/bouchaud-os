#!/usr/bin/env python3
"""Une tache bloquee garde toujours une source de reveil
(P13_SOMMEIL_SANS_SOURCE_V1).

TCG SMP4, banc du dormeur : la boucle de `dort_jusqua` sortait sur l'echeance
passee, la tache encore `Blocked` ; l'echeance etait effacee, et l'attente
n'etait terminee qu'apres la ventilation du reveil. Une preemption ciblee
depuis l'interruption commutait alors une tache `Blocked` sans echeance, hors
de toute file : perdue (detecteur : `pertes_irq=1` a cette etape).

Regles :

  1. `dort_jusqua` : `termine_attente()` AVANT l'effacement de l'echeance qui
     suit la boucle, et plus aucun apres ;
  2. `annule_park_courant` : `state.reveille()` AVANT l'effacement de la cle
     de file et de l'echeance ;
  3. les autres parkings a echeance (`park_current_on_until`,
     `finish_park_current_on_detached`) n'effacent l'echeance qu'apres une
     boucle qui attend la sortie de `Blocked`.

Fail-closed ; tests negatifs par mutation.
"""
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
F = {
    "sommeil": RACINE / "src/kernel/process/thread/sommeil.rs",
    "blocage": RACINE / "src/kernel/process/thread/blocage.rs",
}
TERMINE = "    termine_attente();\n    current().wake_deadline_ns.range(0);\n"


def corps(source: str, signature: str) -> str:
    debut = source.find(signature)
    if debut < 0:
        return ""
    ouverture = source.find("{", source.find(")", debut))
    profondeur = 0
    for i in range(ouverture, len(source)):
        if source[i] == "{":
            profondeur += 1
        elif source[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouverture:i + 1]
    return ""


def verifie(t: dict) -> list:
    fautes = []
    d = corps(t["sommeil"], "fn dort_jusqua(")
    boucle = d.find("while crate::kernel::timer::monotonic_ns() < deadline {")
    apres = d[boucle:] if boucle >= 0 else ""
    if boucle < 0 or TERMINE not in apres:
        fautes.append("dort_jusqua : l'echeance est effacee avant la fin de l'attente (tache Blocked sans source)")
    elif apres.count("termine_attente()") != 1:
        fautes.append("dort_jusqua : l'attente est encore terminee apres l'effacement")
    b = corps(t["blocage"], "pub(crate) fn annule_park_courant(")
    r, k, e = b.find("task.state.reveille();"), b.find("task.wait_queue_key.range(0);"), b.find("task.wake_deadline_ns.range(0);")
    if not (0 <= r < k and r < e):
        fautes.append("annule_park_courant : cle ou echeance effacee avant le reveil")
    for sig, attente in (
        ("pub(crate) fn park_current_on_until(", "while current().state == TaskState::Blocked {"),
        ("pub(crate) fn finish_park_current_on_detached(", "if !blocked {\n            break;"),
    ):
        f = corps(t["blocage"], sig)
        a, e = f.find(attente), f.find("wake_deadline_ns.range(0);")
        if not 0 <= a < e:
            fautes.append(f"{sig.split('fn ')[1][:-1]} : echeance effacee sans attendre la sortie de Blocked")
    return fautes


def negatifs(t: dict) -> list:
    d = corps(t["sommeil"], "fn dort_jusqua(")
    historique = d.replace(TERMINE, "    current().wake_deadline_ns.range(0);\n", 1).replace(
        "    // BOUCHAUD_P15_BROWSER_HANG_FORENSICS\n    forensic_wait_clear(WAIT_SLEEP);\n    true",
        "    termine_attente();\n    // BOUCHAUD_P15_BROWSER_HANG_FORENSICS\n    forensic_wait_clear(WAIT_SLEEP);\n    true", 1)
    mutations = [
        ("ordre historique du sommeil", "sommeil", d, historique),
        ("attente jamais terminee", "sommeil", TERMINE, "    current().wake_deadline_ns.range(0);\n"),
        ("attente terminee deux fois", "sommeil", "    // BOUCHAUD_P15_BROWSER_HANG_FORENSICS\n    forensic_wait_clear(WAIT_SLEEP);\n    true",
         "    termine_attente();\n    // BOUCHAUD_P15_BROWSER_HANG_FORENSICS\n    forensic_wait_clear(WAIT_SLEEP);\n    true"),
        ("annulation dans l'ordre historique", "blocage",
         "    task.state.reveille();\n    task.wait_queue_key.range(0);\n    task.wake_deadline_ns.range(0);",
         "    task.wait_queue_key.range(0);\n    task.wake_deadline_ns.range(0);\n    task.state.reveille();"),
        ("echeance effacee avant l'attente", "blocage",
         "    let notified = crate::kernel::timer::monotonic_ns() < deadline_ns;\n    let task = current();\n    task.wait_queue_key.range(0);\n    task.wake_deadline_ns.range(0);",
         "    let notified = crate::kernel::timer::monotonic_ns() < deadline_ns;\n    let task = current();\n    task.wait_queue_key.range(0);"),
    ]
    echecs = []
    for nom, cle, avant, apres in mutations:
        if not avant or avant not in t[cle] or avant == apres:
            echecs.append(f"mutation inapplicable : {nom}")
            continue
        m = dict(t)
        m[cle] = m[cle].replace(avant, apres, 1)
        if nom == "echeance effacee avant l'attente":
            m[cle] = m[cle].replace("        task.wake_deadline_ns.range(deadline_ns);\n    }\n    if publie_parking(false) == Parking::Gare {\n        arme_echeance(deadline_ns);",
                                    "        task.wake_deadline_ns.range(deadline_ns);\n        task.wake_deadline_ns.range(0);\n    }\n    if publie_parking(false) == Parking::Gare {\n        arme_echeance(deadline_ns);", 1)
        if not verifie(m):
            echecs.append(f"mutation non detectee : {nom}")
    return echecs, len(mutations)


def main() -> int:
    t = {cle: p.read_text(encoding="utf-8") for cle, p in F.items()}
    echecs, n = negatifs(t)
    fautes = verifie(t) + echecs
    for faute in fautes:
        print(f"ECHEC {faute}")
    if fautes:
        return 1
    print(f"tache bloquee jamais sans source de reveil : ok ({n} mutations detectees)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
