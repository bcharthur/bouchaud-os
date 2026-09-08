#!/usr/bin/env python3
"""Garde-fou : la preemption noyau garde ses points surs, et ses gardes.

Le noyau Bouchaud ne commute PAS de pile depuis un contexte d'interruption
arbitraire. Une preemption demandee est donc servie a un POINT SUR : un endroit
ou le code verifie lui-meme qu'il peut rendre la main.

Deux regressions sont possibles, et elles sont opposees.

  * **Perdre les points surs.** Le noyau n'en a longtemps eu qu'UN, au retour
    d'appel systeme. Une tache entree dans le noyau pour une longue operation
    -- un `readv` de trente vecteurs, par exemple -- tenait alors son coeur
    jusqu'a la sortie, quelle que soit l'urgence de ce qui attendait. Retirer
    un point sur ne casse aucun test : cela rend seulement le systeme moins
    reactif, et personne ne le voit.

  * **Perdre les gardes.** `safe_point()` est sur parce qu'il REFUSE de
    commuter dans cinq situations : gros verrou tenu, section critique de rang
    ouverte, preemption desactivee, interruptions coupees, contexte
    d'interruption. C'est ce qui permet d'appeler la fonction n'importe ou.
    Retirer un seul de ces controles transforme chaque site d'appel en
    commutation au mauvais moment -- et le symptome apparait ailleurs, sous
    forme de corruption.

Ce garde-fou tient les deux bouts.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
PREEMPT = RACINE / "src/kernel/scheduler/preempt.rs"

# Les gardes que `safe_point()` doit conserver. Chaque entree nomme ce qu'elle
# empeche, pour que le message d'echec dise pourquoi la ligne existait.
GARDES = [
    ("interrupts_enabled", "commuter les interruptions coupees"),
    ("irq_depth", "commuter depuis un contexte d'interruption"),
    ("preempt_count", "commuter dans une section non preemptible"),
    ("lockdep::depth", "commuter en tenant une section critique de rang"),
    ("held_by_current_cpu", "commuter en tenant le gros verrou"),
]

# Le noyau doit garder au moins ce nombre de sites d'appel HORS du module de
# preemption lui-meme. Le seuil monte quand on en ajoute ; il ne redescend pas.
SITES_MINIMUM = 3


def corps_de_safe_point(source: str) -> str:
    debut = source.find("pub fn safe_point()")
    if debut < 0:
        return ""
    fin = source.find("\npub fn ", debut + 1)
    return source[debut:fin if fin > 0 else len(source)]


def main() -> int:
    fautes = []

    if not PREEMPT.exists():
        print("  - fichier absent : %s" % PREEMPT.relative_to(RACINE).as_posix())
        return 1

    source = PREEMPT.read_text(encoding="utf-8")
    corps = corps_de_safe_point(source)
    if not corps:
        fautes.append("preempt.rs : `safe_point()` a disparu ; plus rien ne sert les preemptions differees.")
    else:
        for marqueur, empeche in GARDES:
            if marqueur not in corps:
                fautes.append(
                    "preempt.rs : `safe_point()` ne verifie plus `%s` ; il pourrait %s."
                    % (marqueur, empeche)
                )

    sites = []
    for chemin in sorted(RACINE.glob("src/**/*.rs")):
        if chemin == PREEMPT:
            continue
        texte = chemin.read_text(encoding="utf-8", errors="replace")
        # Les lignes de commentaire ne sont pas des sites d'appel.
        for ligne in texte.splitlines():
            nu = ligne.strip()
            if nu.startswith("//") or nu.startswith("///"):
                continue
            if re.search(r"\bsafe_point\(\)", nu):
                sites.append(chemin.relative_to(RACINE).as_posix())
                break

    if len(sites) < SITES_MINIMUM:
        fautes.append(
            "preemption : %d fichier(s) appellent `safe_point()`, minimum %d. "
            "Un noyau qui n'a de point sur qu'au retour d'appel systeme ne peut pas "
            "preempter une longue operation ; il la subit jusqu'au bout."
            % (len(sites), SITES_MINIMUM)
        )

    if fautes:
        print("preemption : %d probleme(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1

    print(
        "preemption : %d gardes conserves dans safe_point(), %d fichier(s) porteurs "
        "de points surs" % (len(GARDES), len(sites))
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
