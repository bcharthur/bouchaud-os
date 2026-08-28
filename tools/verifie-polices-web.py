#!/usr/bin/env python3
"""Le Web doit s'afficher dans une police qui a des lettres accentuees.

# Le defaut

Les pages s'affichaient dans SerenitySans : une police d'interface geometrique,
la seule que l'arbre Ladybird epingle embarque, et qui n'a pas la plupart des
lettres accentuees. D'ou l'aspect « cartoon » et les carres vides des captures
d'ecran -- « Avant d'acc[carre]der [carre] Google ».

`tools/ladybird/fontconfig/fonts.conf` corrige pourtant cela depuis longtemps :
ses alias font pointer `sans-serif`, `Arial`, `Helvetica`, `Roboto` et les
autres vers DejaVu. Le fichier etait simplement INSTALLE SOUS CONDITION --
seulement si l'artefact n'en portait pas deja un. Un artefact construit apres
l'introduction du fichier en porte sa propre version, donc la copie du depot
n'avait plus jamais lieu, et toute correction restait sans effet.

# Les trois regles

  1. `run.ps1` installe le fonts.conf du depot SANS CONDITION : le depot est le
     seul endroit ou ce fichier est relu et corrige.
  2. `fonts.conf` declare un repertoire qui CONTIENT celui ou le noyau depose
     les polices (`gui::polices::REPERTOIRE`). Deux chemins qui divergent, et
     fontconfig ne voit rien.
  3. Les generiques CSS et les familles que les vrais sites citent en premier
     designent une police de texte AVANT SerenitySans.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
RUN = RACINE / "run.ps1"
CONF = RACINE / "tools" / "ladybird" / "fontconfig" / "fonts.conf"
POLICES = RACINE / "src" / "gui" / "polices.rs"

# Ce qu'une page reelle demande en premier. Chacune doit avoir un alias.
FAMILLES_ATTENDUES = [
    "sans-serif", "serif", "monospace", "system-ui",
    "Arial", "Helvetica", "Roboto", "Segoe UI",
]


def main() -> int:
    fautes = []

    if not CONF.exists():
        print(f"introuvable : {CONF}")
        return 1
    conf = CONF.read_text(encoding="utf-8")

    # --- 1. installation inconditionnelle ---------------------------------
    if RUN.exists():
        run = RUN.read_text(encoding="utf-8-sig")
        # La faute exacte : un test d'existence qui saute la copie.
        if re.search(r"if\s*\(\s*-not\s*\(\s*Test-Path[^)]*fonts\.conf", run):
            fautes.append(
                "  run.ps1  le fonts.conf du depot n'est copie que si "
                "l'artefact n'en a pas : un artefact recent en porte un, et la "
                "correction du depot n'arrive jamais sur la machine"
            )
        if "fontconfig\\fonts.conf" not in run:
            fautes.append("  run.ps1  le fonts.conf du depot n'est plus installe")

    # --- 2. le repertoire des polices ---------------------------------------
    repertoire = None
    if POLICES.exists():
        trouve = re.search(r'REPERTOIRE:\s*&str\s*=\s*"([^"]+)"',
                           POLICES.read_text(encoding="utf-8"))
        if trouve:
            repertoire = trouve.group(1)
    if repertoire:
        dirs = re.findall(r"<dir>([^<]+)</dir>", conf)
        # fontconfig balaie recursivement : un parent suffit.
        couvert = any(repertoire == d or repertoire.startswith(d.rstrip("/") + "/")
                      for d in dirs)
        if not couvert:
            fautes.append(
                f"  fonts.conf  aucun <dir> ne couvre {repertoire}, ou le noyau "
                f"depose les polices ; fontconfig ne verra rien "
                f"(declares : {', '.join(dirs) or 'aucun'})"
            )

    # --- 3. les alias -------------------------------------------------------
    for famille in FAMILLES_ATTENDUES:
        motif = re.search(
            r"<family>\s*" + re.escape(famille) + r"\s*</family>\s*"
            r"<prefer>(.*?)</prefer>",
            conf, re.S | re.I)
        if not motif:
            fautes.append(
                f"  fonts.conf  `{famille}` n'a pas d'alias : une page qui la "
                f"demande tombera sur la police par defaut"
            )
            continue
        premiers = re.findall(r"<family>([^<]+)</family>", motif.group(1))
        if not premiers:
            fautes.append(f"  fonts.conf  l'alias de `{famille}` est vide")
        elif "serenity" in premiers[0].lower():
            fautes.append(
                f"  fonts.conf  `{famille}` designe SerenitySans en PREMIER : "
                f"c'est la police sans lettres accentuees"
            )

    if fautes:
        print("polices du Web : regle violee")
        print("\n".join(fautes))
        return 1

    print(
        "ok  fonts.conf : installe sans condition, couvre le repertoire du "
        "noyau, et aucun generique ne tombe sur SerenitySans"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
