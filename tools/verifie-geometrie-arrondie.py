#!/usr/bin/env python3
"""Garde-fou : le rectangle arrondi vit a UN seul endroit, et il est total.

# La panique, relevee le 16 septembre 2026

    *** KERNEL PANIC *** cpu=0
    panicked at src/gui/apps/file_explorer.rs:163:17:
    min > max. min = 447, max = 446
    [FAULT] task=3 pid=7 tid=103 nom=desktop

Ouvrir le gestionnaire de fichiers tuait le noyau. La ligne interne de l'icone
de fichier est une boite de trente-quatre de haut avec un rayon de dix-sept :

    cy = py.clamp(y0 + 17, y0 + 34 - 17 - 1) = clamp(haut + 17, haut + 16)

`min > max`, et `clamp` panique. Pour la premiere ligne, `haut` vaut 430 :
447 et 446, les deux nombres du releve.

La MEME primitive existait en deux exemplaires -- ici et dans `reference_gop`
pour le logo de demarrage, peint avant que quoi que ce soit sache rapporter
une faute. Deux copies d'un meme calcul, c'est deux fois la meme faute a
trouver, et une seule a ete trouvee.

# Ce qui est verifie

1. La geometrie est PURE : verifiable sur l'hote.
2. Le rayon est borne par la boite, avec la borne qui ordonne les centres
   d'arc -- `(cote - 1) / 2` et non `cote / 2`.
3. Plus aucun `clamp(x0 + rayon, x1 - rayon - 1)` ailleurs dans l'arbre.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
GEOMETRIE = RACINE / "src/gui/geometrie.rs"
SOURCES = RACINE / "src"


def sans_commentaires(source):
    return "\n".join(
        ligne for ligne in source.splitlines()
        if not ligne.lstrip().startswith("//")
    )


def main():
    fautes = []
    if not GEOMETRIE.exists():
        print("  - fichier absent : %s" % GEOMETRIE)
        return 1

    geo_brut = GEOMETRIE.read_text(encoding="utf-8")
    geo = sans_commentaires(geo_brut)

    # --- 1. La geometrie reste verifiable seule ---------------------------
    for interdit, pourquoi in (
        ("use crate::", "elle depend du noyau"),
        ("unsafe", "elle sort du domaine verifiable"),
    ):
        if interdit in geo:
            fautes.append(
                "geometrie.rs : `%s` est apparu ; %s, donc le fichier ne se "
                "compile plus seul et `tools/gui/test_geometrie_arrondie.rs` "
                "cesserait de couvrir la forme." % (interdit, pourquoi)
            )

    # --- 2. Le rayon est borne, et avec la bonne borne --------------------
    if "pub fn rayon_utilisable(" not in geo:
        fautes.append(
            "geometrie.rs : `rayon_utilisable` a disparu. Sans borne, un rayon "
            "plus grand que la moitie de la boite inverse les centres d'arc et "
            "`clamp` panique -- c'est la panique du 16 septembre."
        )
    elif not re.search(r"\(largeur - 1\)\s*/\s*2", geo) or \
            not re.search(r"\(hauteur - 1\)\s*/\s*2", geo):
        fautes.append(
            "geometrie.rs : la borne du rayon n'est plus `(cote - 1) / 2`. "
            "`cote / 2` ne suffit PAS : avec une hauteur de 34 il rend encore "
            "17, et `clamp(haut + 17, haut + 16)` panique exactement comme "
            "avant. Les bornes de centre d'arc sont INCLUSES des deux cotes."
        )

    # --- 3. Plus aucune copie locale de la primitive ----------------------
    motif = re.compile(
        r"\.clamp\(\s*\w+\s*\+\s*rayon\s*,\s*\w+\s*-\s*rayon\s*-\s*1\s*\)"
    )
    for chemin in sorted(SOURCES.rglob("*.rs")):
        if chemin == GEOMETRIE:
            continue
        texte = sans_commentaires(chemin.read_text(encoding="utf-8"))
        if motif.search(texte):
            fautes.append(
                "%s : une copie locale du rectangle arrondi est reapparue. "
                "C'est exactement la forme qui panique quand la boite fait deux "
                "fois le rayon, et elle existait en DEUX exemplaires -- dont "
                "celui du logo de demarrage, peint avant que quoi que ce soit "
                "sache rapporter une faute. Employer `gui::geometrie::"
                "dans_arrondi`, qui est totale."
                % chemin.relative_to(RACINE)
            )

    if fautes:
        print("geometrie arrondie : %d probleme(s)" % len(fautes))
        for faute in fautes:
            print("\n  - %s" % faute)
        return 1

    print(
        "geometrie arrondie : une seule implementation, pure, rayon borne par "
        "la boite, aucune copie locale"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
