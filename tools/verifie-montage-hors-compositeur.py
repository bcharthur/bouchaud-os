#!/usr/bin/env python3
"""Garde-fou : le montage du disque ne revient jamais dans le chemin de rendu.

# Le defaut que cette regle interdit

`execute_le_montage_differe()` sonde le disque interne : table GPT, partitions,
persistance. C'est une operation qui peut durer, et qui descend loin dans la
pile. Elle etait appelee EN LIGNE depuis la boucle de trames du gestionnaire de
fenetres -- donc sur la pile du compositeur, dans son quantum, entre deux
images.

Une premiere correction l'a deplacee dans son propre fil, mais avec un REPLI en
ligne quand le fil ne pouvait pas etre cree. Ce repli annulait la garantie au
moment ou elle comptait le plus : ne pas pouvoir creer une tache veut dire
memoire sous pression ou table des processus pleine, c'est-a-dire un systeme
deja degrade -- le pire moment pour figer le fil graphique.

# Ce qui est verifie

  1. `execute_le_montage_differe()` n'est appele QUE depuis le corps du fil de
     montage. Aucun appel depuis le gestionnaire de fenetres, le compositeur,
     le chemin d'entree, une interruption ou le rendu.
  2. `lance_le_montage_differe()` ne contient PAS d'appel de repli.
  3. Le refus est COMPTE : une persistance non montee alors qu'elle etait
     demandee ne doit pas etre silencieuse.

# Ce que cette separation ne fait PAS, et qu'il ne faut pas ecrire

Elle protege la latence et la pile du compositeur. Elle ne protege pas le noyau
d'une faute fatale : un double fault en anneau zero dans le fil de montage tue
le systeme entier, exactement comme dans le bureau. La regle refuse donc aussi
qu'on documente le contraire.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
INSTALL = RACINE / "src/platform/pc/installation.rs"

# Les fichiers d'ou l'appel direct est interdit.
INTERDITS = [
    "src/gui/window_manager.rs",
    "src/gui/composited.rs",
    "src/gui/composited_corps.rs",
    "src/gui/desktop.rs",
    "src/gui/frame_clock.rs",
    "src/gui/scene.rs",
    "src/arch/x86_64/idt/timer.rs",
    "src/arch/x86_64/idt/exceptions.rs",
    "src/drivers/input/ps2_keyboard.rs",
]

# Une affirmation d'isolation qui serait fausse.
MENSONGES = [
    "isole le noyau",
    "isole la faute",
    "protege le noyau d'une faute",
]


def corps(source: str, entete: str):
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
                return source[debut:j + 1]
    return None


def main() -> int:
    fautes = []
    if not INSTALL.exists():
        print("  - fichier absent : src/platform/pc/installation.rs")
        return 1
    install = INSTALL.read_text(encoding="utf-8")

    lance = corps(install, "pub fn lance_le_montage_differe(")
    if lance is None:
        fautes.append(
            "installation.rs : lance_le_montage_differe() a disparu ; le bureau "
            "n'a plus de point de declenchement asynchrone."
        )
    else:
        if "spawn_noyau" not in lance:
            fautes.append(
                "installation.rs : le montage ne part plus dans son propre fil."
            )
        if "execute_le_montage_differe()" in lance:
            fautes.append(
                "installation.rs : lance_le_montage_differe() retombe sur un montage "
                "EN LIGNE. Ne pas pouvoir creer une tache veut dire systeme deja "
                "degrade -- c'est le pire moment pour figer le fil graphique."
            )
        if "MONTAGES_REFUSES" not in lance:
            fautes.append(
                "installation.rs : un refus de lancement n'est plus compte ; la "
                "persistance disparaitrait en silence sous pression memoire."
            )

    fil = corps(install, "fn fil_de_montage(")
    if fil is None or "execute_le_montage_differe()" not in fil:
        fautes.append(
            "installation.rs : le fil de montage n'execute plus le montage ; il "
            "serait demande et jamais fait."
        )

    appel = re.compile(r"\bexecute_le_montage_differe\s*\(")
    for relatif in INTERDITS:
        chemin = RACINE / relatif
        if not chemin.exists():
            continue
        texte = chemin.read_text(encoding="utf-8")
        # Les commentaires ne comptent pas : ce sont les APPELS qui figent une
        # trame, pas les phrases qui en parlent.
        code = "\n".join(l for l in texte.splitlines() if not l.strip().startswith("//"))
        if appel.search(code):
            fautes.append(
                "%s : appel direct a execute_le_montage_differe(). Ce chemin rend "
                "des images ou sert des interruptions ; une sonde disque n'y a pas "
                "sa place." % relatif
            )

    minuscule = install.lower()
    for mensonge in MENSONGES:
        if mensonge in minuscule:
            fautes.append(
                "installation.rs : la documentation affirme « %s ». Deplacer une "
                "faute noyau d'un fil vers un autre ne l'isole pas : un double "
                "fault en anneau zero tue le systeme depuis n'importe quel fil."
                % mensonge
            )

    if fautes:
        print("montage hors compositeur : %d probleme(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1

    print(
        "montage hors compositeur : lance dans un fil, aucun repli en ligne, refus "
        "compte, aucun appel direct depuis le rendu ou les interruptions"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
