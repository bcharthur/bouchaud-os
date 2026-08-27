#!/usr/bin/env python3
"""Rendre les polices systeme de Bouchaud visibles a Ladybird.

LE CONSTAT
----------
L'arbre Ladybird epingle n'embarque que DEUX polices, et une seule de texte :

    Base/res/fonts/SerenitySans-Regular.ttf
    Base/res/fonts/NotoEmoji.ttf

Le portage forcait donc `set_system_font_family("SerenitySans")`, et la
configuration fontconfig faisait pointer toutes les familles generiques -- y
compris `monospace` -- vers cette meme police. Ce n'etait pas un raccourci de
bootstrap : c'etait la description exacte de ce que la machine savait afficher.

Deux consequences visibles :

  * tout le Web s'affichait dans une police d'interface geometrique, d'ou
    l'aspect « cartoon » ;
  * `<pre>`, `<code>` et `font-family: monospace` etaient rendus en
    PROPORTIONNEL, ce qui desaligne tout code affiche sur une page.

CE QUE BOUCHAUD A DEJA
----------------------
Le noyau depose DejaVu Sans, DejaVu Sans Bold, DejaVu Sans Mono et DejaVu Sans
Mono Bold dans `/usr/share/fonts/truetype/dejavu` (`kernel::sysroot::install_fonts`).
Ladybird ne les voyait pas, pour une raison precise : son fournisseur de polices
ne lit que `resource://fonts`, c'est-a-dire l'arborescence de l'artefact.

CE QUE FAIT CE SCRIPT
---------------------
Il ajoute le repertoire systeme au fournisseur de chemins, en plus -- jamais a
la place -- des ressources de l'artefact. Les deux chemins de resolution voient
alors les memes familles :

  * `Gfx::FontDatabase` / PathFontProvider, qui sert `system_font_family()` et
    les familles nommees ;
  * fontconfig, qui sert le repli de Skia (voir `fontconfig/fonts.conf`).

Le repertoire est ajoute meme s'il est absent : `load_all_fonts_from_uri` sur un
chemin vide ne charge rien et ne rompt rien. C'est ce qui permet a la meme image
de demarrer avec ou sans polices systeme, sans condition a l'execution.
"""

import sys
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else ".")

REPERTOIRE_SYSTEME = "/usr/share/fonts"

ancre = '    font_provider.load_all_fonts_from_uri("resource://fonts"sv);'

remplacement = '''    font_provider.load_all_fonts_from_uri("resource://fonts"sv);
#if defined(BOUCHAUD_PORT)
    // BOUCHAUD : les polices du systeme, en PLUS de celles de l'artefact.
    // Le noyau depose DejaVu dans /usr/share/fonts ; sans cette ligne, seul
    // fontconfig les voyait, et `system_font_family()` restait limite a
    // l'unique police de texte de l'arbre epingle. Voir
    // tools/ladybird/prepare-fonts-systeme.py.
    font_provider.load_all_fonts_from_uri("file:///usr/share/fonts"sv);
    outln("[ladybird-bouchaud] FONTS_SYSTEM_DIR /usr/share/fonts");
#endif'''

chemin = root / "Services/WebContent/main.cpp"
data = chemin.read_text()

if "FONTS_SYSTEM_DIR" in data:
    print("polices systeme : deja applique")
    raise SystemExit(0)

if ancre not in data:
    raise SystemExit(
        "polices systeme : ancre `load_all_fonts_from_uri` introuvable dans "
        "Services/WebContent/main.cpp"
    )

chemin.write_text(data.replace(ancre, remplacement, 1))
print("polices systeme ajoutees au fournisseur de chemins:", REPERTOIRE_SYSTEME)
