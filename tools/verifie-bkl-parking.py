#!/usr/bin/env python3
"""Aucun chemin d'attente du gros verrou ne doit tourner a vide.

# Pourquoi cette regle existe

Bouchaud tourne sous TCG : les quatre vCPU se partagent les coeurs de l'hote.
Un vCPU qui tourne a vide en attendant le verrou ne se contente pas de perdre
son temps -- il VOLE le temps dont le detenteur a besoin pour finir. Plus les
autres attendent, plus il tient. C'est la pathologie classique du detenteur
preempte, et elle se voyait telle quelle dans le runtime :

    [SMP-PROV] owner=1 held=690ms depth=1 syscall=poll/attente
    [BKL-MAX-HOLD] ns=29562372510 origine=resume_after_schedule
    window_ns=11353070412        <- une fenetre de 5 s en a pris 11
    [gui] client actif 0 trames (silence 61818 ms)

`resume_after_schedule` etait un `spin_loop()` pur. Sa justification -- « avec
des IPI cibles, plus aucun battement ne garantit le reveil » -- ne tenait pas :
`wake_parked_waiters` est appele par CHAQUE liberation.

# Les deux regles

  1. `resume_after_schedule` attend par `wait_for_owner_change`, qui garde un
     court spin actif puis se gare.
  2. Les deux chemins de liberation -- `release_one` et `suspend_for_schedule`
     -- reveillent les gares. Sans cela, se garer serait un blocage definitif,
     et la regle 1 deviendrait dangereuse.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
BKL = RACINE / "src" / "kernel" / "sync" / "bkl.rs"
BKL_TREE = RACINE / "src" / "kernel" / "sync" / "bkl"

def source_de(*chemins) -> str:
    """Le code d'un sous-systeme, quel que soit son decoupage en fichiers.

    Ce garde-fou lisait un seul fichier. La fragmentation de `bkl.rs` en
    `bkl/**` l'a fait tomber sur une exception -- et comme rien ne l'executait,
    la regle a cesse de proteger quoi que ce soit sans que personne le voie.
    Lire un ARBRE plutot qu'un fichier retire cette facon de casser.
    """
    morceaux = []
    for chemin in chemins:
        if chemin.is_dir():
            for fichier in sorted(chemin.rglob("*.rs")):
                morceaux.append(fichier.read_text(encoding="utf-8"))
        elif chemin.exists():
            morceaux.append(chemin.read_text(encoding="utf-8"))
    return "\n".join(morceaux)



def corps(source: str, signature: str) -> str:
    debut = source.index(signature)
    profondeur = 0
    for position in range(debut, len(source)):
        if source[position] == "{":
            profondeur += 1
        elif source[position] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[debut : position + 1]
    raise SystemExit(f"corps introuvable : {signature}")


def main() -> int:
    source = source_de(BKL, BKL_TREE)
    fautes = []

    resume = corps(source, "pub fn resume_after_schedule(")
    if "wait_for_owner_change(" not in resume:
        fautes.append(
            "  resume_after_schedule attend sans se garer : sous TCG, un vCPU "
            "qui tourne a vide vole le temps du detenteur, qui tient donc plus "
            "longtemps. Passer par `wait_for_owner_change`."
        )
    # Un `spin_loop()` nu comme SEULE attente est precisement la faute. Il en
    # reste un legitime dans `wait_for_owner_change` lui-meme.
    boucle = resume[resume.index("loop {"):] if "loop {" in resume else ""
    if re.search(r"^\s*spin_loop\(\);\s*$", boucle, re.M) and \
            "wait_for_owner_change(" not in boucle:
        fautes.append(
            "  resume_after_schedule garde un `spin_loop()` nu dans sa boucle "
            "d'attente"
        )

    for fonction, nom in [("fn release_one(", "release_one"),
                          ("pub fn suspend_for_schedule(", "suspend_for_schedule")]:
        if "wake_parked_waiters(" not in corps(source, fonction):
            fautes.append(
                f"  {nom} ne reveille plus les CPU gares : se garer sur ce "
                f"verrou deviendrait un blocage definitif"
            )

    # Le protocole de publication qui ferme la course du reveil perdu. C'est
    # son ORDRE qui le rend correct, pas la presence de ses morceaux : publier
    # l'attente APRES avoir relu le proprietaire ne protege de rien.
    attente = corps(source, "fn wait_for_owner_change(")
    etapes = [
        ("prepare_lock_park", "armement"),
        ("PARKED.fetch_or", "publication de l'attente"),
        ("owner_load(Ordering::SeqCst)", "relecture atomique du proprietaire"),
        ("commit_lock_park", "sommeil"),
    ]
    positions = []
    for element, role in etapes:
        ou = attente.find(element)
        if ou == -1:
            fautes.append(
                f"  wait_for_owner_change a perdu son {role} (`{element}`) : "
                f"le reveil perdu redevient possible"
            )
        positions.append((ou, role))
    if all(ou != -1 for ou, _ in positions):
        for (avant_ou, avant_role), (apres_ou, apres_role) in zip(positions, positions[1:]):
            if avant_ou > apres_ou:
                fautes.append(
                    f"  wait_for_owner_change  {apres_role} precede "
                    f"{avant_role} : c'est l'ordre total des quatre acces qui "
                    f"interdit le reveil perdu, pas leur presence"
                )

    if fautes:
        print("attente du gros verrou : regle violee")
        print("\n".join(fautes))
        return 1

    print(
        "ok  src/kernel/sync/bkl.rs : toute attente du verrou se gare, et "
        "chaque liberation reveille les gares"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
