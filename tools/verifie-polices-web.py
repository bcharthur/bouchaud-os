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
    # Les polices de MARQUE, citees en PREMIER par les sites qui en ont une --
    # et c'est la premiere qui decide. Sans alias, Ladybird retombe sur sa
    # police par defaut : c'est ce qui laissait « FR », « Se connecter » et
    # « Tout accepter » en lettres geometriques sur une page dont le corps de
    # texte etait deja correct.
    "Google Sans", "Product Sans", "Open Sans", "Inter",
]


def substitutions_de(conf: str, famille: str) -> list[str] | None:
    """Vers quoi `famille` est redirigee, quelle que soit la forme employee.

    Rend `None` si aucune regle ne la mentionne, et la liste des familles
    cibles sinon -- la premiere etant celle qui decide.
    """
    alias = re.search(
        r"<family>\s*" + re.escape(famille) + r"\s*</family>\s*<prefer>(.*?)</prefer>",
        conf, re.S | re.I)
    if alias:
        return re.findall(r"<family>([^<]+)</family>", alias.group(1))

    correspondance = re.search(
        r"<match[^>]*target=\"pattern\"[^>]*>(?:(?!</match>).)*?"
        r"<test[^>]*name=\"family\"[^>]*>\s*<string>\s*"
        + re.escape(famille) +
        r"\s*</string>\s*</test>(?:(?!</match>).)*?</match>",
        conf, re.S | re.I)
    if not correspondance:
        return None
    bloc = correspondance.group(0)
    edition = re.search(
        r"<edit[^>]*name=\"family\"[^>]*>(.*?)</edit>", bloc, re.S | re.I)
    if not edition:
        return []
    return re.findall(r"<string>([^<]+)</string>", edition.group(1))


def main() -> int:
    fautes = []

    if not CONF.exists():
        print(f"introuvable : {CONF}")
        return 1
    conf = CONF.read_text(encoding="utf-8")

    # --- 0. le fichier doit etre du XML VALIDE ------------------------------
    #
    # fontconfig qui ne peut pas lire sa configuration n'en charge AUCUNE : ni
    # repertoires, ni alias. Toute la page retombe alors sur la police par
    # defaut, et le seul indice est une ligne d'avertissement noyee dans le
    # journal. Un `--` egare dans un commentaire suffit -- XML l'interdit --,
    # et c'est exactement ce qui a failli passer en ajoutant les alias de
    # marque.
    try:
        import xml.dom.minidom
        xml.dom.minidom.parseString(conf)
    except Exception as erreur:  # noqa: BLE001
        fautes.append(
            f"  fonts.conf  XML invalide ({erreur}) : fontconfig ne chargerait "
            f"AUCUNE configuration, et tout le Web retomberait sur la police "
            f"par defaut"
        )

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

    # --- 3. les substitutions ----------------------------------------------
    #
    # Fontconfig sait exprimer une substitution de DEUX facons, et les deux
    # marchent :
    #
    #   <alias><family>X</family><prefer><family>Y</family></prefer></alias>
    #   <match target="pattern">
    #     <test name="family" compare="eq"><string>X</string></test>
    #     <edit name="family" mode="assign"><string>Y</string></edit>
    #   </match>
    #
    # Ce garde-fou n'en connaissait qu'une. Le passage a la seconde -- plus
    # forte, parce que `binding="strong"` gagne contre la liste CSS du site --
    # l'a fait declarer absents des alias qui existaient bel et bien. Un
    # garde-fou qui accuse a tort finit desactive ; il doit donc lire les deux.
    for famille in FAMILLES_ATTENDUES:
        cibles = substitutions_de(conf, famille)
        if cibles is None:
            fautes.append(
                f"  fonts.conf  `{famille}` n'a aucune substitution : une page "
                f"qui la demande tombera sur la police par defaut"
            )
            continue
        if not cibles:
            fautes.append(f"  fonts.conf  la substitution de `{famille}` est vide")
        elif "serenity" in cibles[0].lower():
            fautes.append(
                f"  fonts.conf  `{famille}` designe SerenitySans en PREMIER : "
                f"c'est la police sans lettres accentuees"
            )

    # --- 4. les graisses intermediaires -------------------------------------
    #
    # Un bouton de Google demande `font-weight: 500`. DejaVu n'a que Book et
    # Bold : sans regle de rabattement, fontconfig peut ne trouver aucune
    # correspondance et rendre la main a la police par defaut.
    if "compare=\"more_eq\"" not in conf or "weight" not in conf:
        fautes.append(
            "  fonts.conf  aucune regle ne rabat les graisses intermediaires "
            "(500, 600) sur celles que DejaVu possede : les boutons d'un site "
            "retomberaient sur la police par defaut"
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
