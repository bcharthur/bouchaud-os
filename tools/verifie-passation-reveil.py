#!/usr/bin/env python3
"""Passation et reveil : une barriere totale de chaque cote.

BOUCHAUD_PASSATION_REVEIL_ORDONNES_V1

Une tache bloquee reveillee pendant sa commutation de sortie est publiee par
la passation (qui ecrit `on_cpu = -1`, `switching_out = false`, puis lit
l'etat) OU par le reveilleur (qui ecrit l'etat, puis lit `on_cpu` /
`switching_out`). Motif « store buffer » : sans barriere `SeqCst` entre
l'ecriture et la lecture, x86-TSO laisse les deux lectures passer devant les
deux ecritures, et personne ne publie la tache -- `[SCHED-ORPHELINE]`,
scheduler-ng-banc SMP4, banc fige.

Le garde exige :
  1. dans `complete_switch_handoff`, la barriere APRES `switching_out = false`
     et AVANT la lecture de l'etat ;
  2. dans `publish_ready`, la barriere AVANT la premiere lecture ;
  3. le banc hote `tools/smp/test_passation_reveil.rs` (modele x86-TSO
     exhaustif, ancien et nouveau protocole).

Fail-closed ; trois tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
COURANT = "src/kernel/process/thread/courant.rs"
CREATION = "src/kernel/process/thread/creation.rs"
TEST = "tools/smp/test_passation_reveil.rs"
FICHIERS = (COURANT, CREATION, TEST)
BARRIERE = "core::sync::atomic::fence(Ordering::SeqCst);"


def sans_commentaires(texte: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in texte.splitlines())


def corps(texte: str, nom: str) -> str:
    m = re.search(r"\bfn\s+" + re.escape(nom) + r"\s*\(", texte)
    if not m:
        return ""
    ouverture = texte.find("{", m.end())
    profondeur = 0
    for i in range(ouverture, len(texte)):
        if texte[i] == "{":
            profondeur += 1
        elif texte[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return texte[ouverture:i + 1]
    return ""


def verifie(racine: Path) -> list[str]:
    try:
        src = {f: sans_commentaires((racine / f).read_text(encoding="utf-8")) for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    fautes = []
    h = corps(src[COURANT], "complete_switch_handoff")
    i, j, k = h.find("task.switching_out.range(false);"), h.find(BARRIERE), h.find("task.state == TaskState::Ready")
    if min(i, j, k) < 0 or not i < j < k:
        fautes.append(f"{COURANT} : la passation lit l'etat sans barriere apres ses ecritures")
    p = corps(src[CREATION], "publish_ready")
    b, l = p.find(BARRIERE), p.find("tasks()[index].state")
    if b < 0 or l < 0 or b > l:
        fautes.append(f"{CREATION} : publish_ready lit on_cpu/switching_out sans barriere")
    if "fn explore(nouveau: bool)" not in src[TEST] or "modele_tso_l_ancien_protocole_perd_la_tache" not in src[TEST]:
        fautes.append(f"{TEST} : le modele x86-TSO a disparu")
    return fautes


def mutation(fichier: str, avant: str, apres: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        copie = Path(tmp)
        for f in FICHIERS:
            dest = copie / f
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text((RACINE / f).read_text(encoding="utf-8"), encoding="utf-8")
        cible = copie / fichier
        texte = cible.read_text(encoding="utf-8")
        if avant not in texte:
            return False
        cible.write_text(texte.replace(avant, apres, 1), encoding="utf-8")
        return bool(verifie(copie))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("passation/reveil : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        (COURANT, "        core::sync::atomic::fence(Ordering::SeqCst);\n        task.state == TaskState::Ready",
         "        task.state == TaskState::Ready"),
        (CREATION, "    core::sync::atomic::fence(Ordering::SeqCst);\n    if index >= tasks().len()",
         "    if index >= tasks().len()"),
        (TEST, "fn explore(nouveau: bool)", "fn explore_(nouveau: bool)"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"passation/reveil : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"PASSATION_REVEIL_OK negatifs={len(negatifs)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
