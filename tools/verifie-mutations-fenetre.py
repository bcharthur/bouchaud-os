#!/usr/bin/env python3
"""La geometrie canonique d'une fenetre n'est mutee que par la politique.

# Ce que ce garde-fou empeche

`x`, `y`, `w`, `h`, `min`, `placement` et `restore_rect` decrivent l'etat d'une
fenetre. Le compositeur en deduit ce qu'il peint ET ce qu'il invalide : une
mutation faite ailleurs que dans la politique change l'image sans que personne
n'annonce de degat, et les pixels de l'etat precedent restent a l'ecran.

Trois fichiers ont le droit de muter cet etat :

  * `windowing/manager.rs` et `windowing/state.rs` -- la politique elle-meme ;
  * `window_manager.rs` et `window.rs` -- les deux adaptateurs d'execution,
    audites, qui accompagnent chaque mutation d'un degat.

Tout le reste de `src/gui` doit passer par une `WindowCommand`.

# Pourquoi en Python

La version d'origine etait un script shell appuye sur `rg`. `validate-fast`
tourne sous PowerShell, ou ni `bash` ni ripgrep ne sont garantis : le garde-fou
ne se serait pas execute la ou il compte. Le reste des verificateurs du depot
est en Python, sans dependance ; celui-ci les rejoint.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
GUI = RACINE / "src" / "gui"

# Les deux implementations de la politique, et les deux adaptateurs audites.
AUTORISES = {
    "windowing/manager.rs",
    "windowing/state.rs",
    "window_manager.rs",
    "window.rs",
}

CHAMPS = ("x", "y", "w", "h", "min", "placement", "restore_rect")

# `w.x = ...`, `top.h += ...`, `window.placement = ...`
MUTATION = re.compile(
    r"\b(?:w|top|window)\.(?:" + "|".join(CHAMPS) + r")\s*(?:=[^=]|\+=|-=)"
)


def main() -> int:
    violations = []
    for chemin in sorted(GUI.rglob("*.rs")):
        relatif = chemin.relative_to(GUI).as_posix()
        if relatif in AUTORISES:
            continue
        for numero, ligne in enumerate(
            chemin.read_text(encoding="utf-8").splitlines(), 1
        ):
            if MUTATION.search(ligne):
                violations.append(f"  {relatif}:{numero}  {ligne.strip()}")

    if violations:
        print("mutation d'etat de fenetre hors adaptateur audite :")
        print("\n".join(violations))
        print(
            "\nPasser par une WindowCommand : la politique mute l'etat, "
            "l'adaptateur annonce le degat."
        )
        return 1

    print(
        f"ok  src/gui : l'etat de fenetre n'est mute que par "
        f"{len(AUTORISES)} fichiers audites"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
