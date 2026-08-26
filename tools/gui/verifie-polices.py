#!/usr/bin/env python3
"""Le manifeste des polices dit-il la verite ?

## Ce que cette barriere attrape

Une police manquante ne produit AUCUNE erreur. Le texte tombe silencieusement
sur une autre famille, et la seule trace est visuelle -- un `<pre>` qui n'est
plus aligne, une graisse qui n'apparait jamais. C'est le genre de defaut qu'on
met des mois a remarquer.

Trois accords sont verifies :

  1. chaque police declaree existe reellement dans `src/assets/fonts/` ;
  2. chaque fichier de `src/assets/fonts/` est declare -- c'est ce qui manquait
     pour `DejaVuSansMono-Bold.ttf`, embarque dans le binaire et jamais
     installe ;
  3. chaque famille preferee par la configuration fontconfig du portage
     Ladybird est reellement fournie par le manifeste.

Le troisieme point lit `tools/ladybird/fontconfig/fonts.conf` SANS le modifier.

Code de retour : 0 si tout concorde.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[2]
MANIFESTE = RACINE / "src" / "gui" / "polices.rs"
ASSETS = RACINE / "src" / "assets" / "fonts"
FONTCONFIG = RACINE / "tools" / "ladybird" / "fontconfig" / "fonts.conf"

# Familles citees par fontconfig dont l'absence est CONNUE et datee.
#
# `DejaVu Serif` a ete introduite par le commit « fix(fonts): let Ladybird see
# the system text fonts » sans que l'asset correspondant soit ajoute. Elle
# retombe sur `DejaVu Sans`, cite juste apres dans la meme chaine, donc rien
# n'est casse -- mais la configuration affirme quelque chose de faux.
#
# Le fichier vit sous `tools/ladybird/`, gele pendant la construction de
# l'artefact du run 32961550131. A retirer des que ce gel est leve : soit en
# ajoutant vraiment DejaVu Serif et sa licence, soit en supprimant l'alias.
ABSENCES_CONNUES = {"DejaVu Serif"}


def familles_du_manifeste() -> set[str]:
    source = MANIFESTE.read_text(encoding="utf-8")
    bloc = re.search(r"FAMILLES_FOURNIES: \[&str; \d+\] = \[(.*?)\];", source, re.S)
    if not bloc:
        raise SystemExit("polices : FAMILLES_FOURNIES introuvable dans polices.rs")
    return set(re.findall(r'"([^"]+)"', bloc.group(1)))


def fichiers_du_manifeste() -> set[str]:
    source = MANIFESTE.read_text(encoding="utf-8")
    return set(re.findall(r'fichier:\s*"([^"]+)"', source))


def fichiers_declares_en_octets() -> set[str]:
    source = MANIFESTE.read_text(encoding="utf-8")
    inclus = set(re.findall(r'include_bytes!\("\.\./assets/fonts/([^"]+)"\)', source))
    # `DejaVuSans.ttf` passe par `gui::font::FONT_DATA` plutot que par un
    # `include_bytes!` direct : c'est la meme police, chargee une seule fois
    # pour le rasterizer du bureau.
    if "FONT_DATA" in source:
        police = RACINE / "src" / "gui" / "font.rs"
        for nom in re.findall(
            r'FONT_DATA: &\[u8\] = include_bytes!\("\.\./assets/fonts/([^"]+)"\)',
            police.read_text(encoding="utf-8"),
        ):
            inclus.add(nom)
    return inclus


def familles_preferees_par_fontconfig() -> set[str]:
    if not FONTCONFIG.exists():
        return set()
    source = FONTCONFIG.read_text(encoding="utf-8")
    familles = set()
    for bloc in re.findall(r"<prefer>(.*?)</prefer>", source, re.S):
        familles.update(re.findall(r"<family>([^<]+)</family>", bloc))
    # SerenitySans et Noto Emoji viennent de l'artefact Ladybird, pas de nos
    # assets : leur presence est la responsabilite de l'arbre epingle.
    return {f for f in familles if f.startswith("DejaVu")}


def main() -> int:
    echecs: list[str] = []

    sur_disque = {f.name for f in ASSETS.glob("*.ttf")}
    declares = fichiers_du_manifeste()
    inclus = fichiers_declares_en_octets()

    print("-- manifeste contre assets --")
    for nom in sorted(declares | sur_disque):
        if nom not in sur_disque:
            echecs.append(f"{nom} est declare mais absent de src/assets/fonts/")
        elif nom not in declares:
            echecs.append(
                f"{nom} existe dans src/assets/fonts/ mais n'est declare nulle part "
                "-- il ne sera jamais installe"
            )
        else:
            print(f"  ok     {nom}")

    print("\n-- chaque police declaree est bien embarquee --")
    for nom in sorted(declares):
        if nom not in inclus:
            echecs.append(f"{nom} est declare mais aucun include_bytes! ne le charge")
        else:
            print(f"  ok     {nom} embarque")

    print("\n-- fontconfig contre manifeste --")
    fournies = familles_du_manifeste()
    for famille in sorted(familles_preferees_par_fontconfig()):
        if famille in fournies:
            print(f"  ok     {famille}")
        elif famille in ABSENCES_CONNUES:
            print(f"  connu  {famille} : absence documentee, voir ABSENCES_CONNUES")
        else:
            echecs.append(
                f"fontconfig prefere « {famille} », qu'aucune police installee ne fournit "
                "-- le texte tombera en silence sur la famille suivante"
            )

    if echecs:
        print()
        for echec in echecs:
            print(f"  ECHEC  {echec}")
        print(
            "\nLe manifeste, les assets et la configuration ne disent pas la meme chose."
            "\nVoir src/gui/polices.rs : c'est lui qui fait foi."
        )
        return 1

    print(f"\n{len(declares)} polices declarees, installees et coherentes.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
