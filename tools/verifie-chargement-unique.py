#!/usr/bin/env python3
"""Un seul indicateur de chargement par page : celui du chrome Ladybird.

BOUCHAUD_P19_BROWSER_LOADING_SINGLE_OWNER_V1 (ONE-LOADER, P19, `8e1bdf97`)

Captures P19 : DEUX barres sur une page active -- celle du chrome
(`BouchaudChrome.h`, sous l'URL) et une jauge du NOYAU, posee par le bureau
par-dessus les pixels de WebContent (`widgets.rs::dessine_jauge`), qui
mesurait des rafales de trames et repartait sur une simple animation. Le
correctif garde la jauge noyau pour le DEMARRAGE de la fenetre seulement.

Ce garde verifie la forme qui porte ce contrat :

  1. `compose_client` ne dessine plus rien APRES la recopie de la surface du
     client (plus de `dessine_jauge`, ni d'autre jauge) ; l'ecran de
     demarrage reste dessine, et rend la main, quand `Etat::Demarrage` ;
  2. la duree en clair du titre n'est dessinee qu'en `Etat::Demarrage` ;
  3. le bureau ne salit la zone de la jauge, et ne la dit visible, qu'en
     `Etat::Demarrage` ;
  4. le chrome Ladybird porte l'indicateur unique (`BOUCHAUD_P19_LOADING_UNIFIED_V1`).

Niveau de preuve : STATIQUE. La preuve de rendu (une seule barre sur une page
active, capture QEMU) reste a faire.

Fail-closed ; quatre tests negatifs.
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
WIDGETS = "src/gui/widgets.rs"
WM = "src/gui/window_manager.rs"
CHROME = "tools/ladybird/chrome/BouchaudChrome.h"
FICHIERS = (WIDGETS, WM, CHROME)


def sans_commentaires(t: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in t.splitlines())


def corps(texte: str, nom: str) -> str:
    m = re.search(r"\bfn\s+" + re.escape(nom) + r"\s*\(", texte)
    if not m:
        return ""
    o = texte.find("{", m.end())
    p = 0
    for i in range(o, len(texte)):
        p += texte[i] == "{"
        p -= texte[i] == "}"
        if p == 0:
            return texte[o:i + 1]
    return ""


def verifie(racine: Path) -> list[str]:
    try:
        brut = {f: (racine / f).read_text(encoding="utf-8") for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    w = sans_commentaires(brut[WIDGETS])
    wm = sans_commentaires(brut[WM])
    fautes = []

    if re.search(r"\bdessine_jauge\s*\(", w):
        fautes.append(f"{WIDGETS} : `dessine_jauge` est revenue (jauge noyau sur les pixels de WebContent)")
    c = corps(w, "compose_client")
    if not c:
        fautes.append(f"{WIDGETS} : compose_client introuvable")
    else:
        m = re.search(r"if client\.etat == Etat::Demarrage \{\s*dessine_demarrage\([^;]*\);\s*return;\s*\}", c)
        if not m:
            fautes.append(f"{WIDGETS} : l'ecran de demarrage n'est plus dessine-puis-rendu en Etat::Demarrage")
        elif re.search(r"jauge", c[m.end():]):
            fautes.append(f"{WIDGETS} : compose_client touche une jauge apres la recopie de la surface")

    d = corps(w, "draw_window")
    i = d.find("dessine_duree_chargement(")
    if i < 0:
        fautes.append(f"{WIDGETS} : la duree de demarrage du titre a disparu de draw_window")
    elif "Etat::Demarrage" not in d[max(0, i - 200):i]:
        fautes.append(f"{WIDGETS} : la duree du titre est dessinee hors Etat::Demarrage")

    if not re.search(r"if jauge_change && demarrage \{\s*degats\.ajoute\(Origine::Client, zone\);", wm):
        fautes.append(f"{WM} : la zone de la jauge est salie hors du demarrage")
    if "jauge_visible |= demarrage && client.jauge.visible();" not in wm:
        fautes.append(f"{WM} : la jauge est dite visible hors du demarrage")
    if "client.etat == crate::gui::client::Etat::Demarrage && client.jauge.visible()" not in wm:
        fautes.append(f"{WM} : le chemin sans degats rend la jauge visible hors du demarrage")

    if brut[CHROME].count("BOUCHAUD_P19_LOADING_UNIFIED_V1") < 2:
        fautes.append(f"{CHROME} : l'indicateur unique du chrome n'est plus marque")
    return fautes


def mutation(fichier: str, a: str, b: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        for f in FICHIERS:
            d = Path(tmp) / f
            d.parent.mkdir(parents=True, exist_ok=True)
            t = (RACINE / f).read_text(encoding="utf-8")
            if f == fichier:
                if a not in t:
                    return False
                t = t.replace(a, b, 1)
            d.write_text(t, encoding="utf-8")
        return bool(verifie(Path(tmp)))


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("chargement unique : regle violee")
        print("\n".join("  " + f for f in fautes))
        return 1
    negatifs = [
        # 1. La jauge noyau reposee sur la surface du client.
        (WIDGETS, "    fb::note_pixels_dessines(((x_fin - x_debut) * (y_fin - y_debut)) as u64);\n",
         "    fb::note_pixels_dessines(((x_fin - x_debut) * (y_fin - y_debut)) as u64);\n"
         "    dessine_jauge(&zone, &client.jauge, maintenant);\n"),
        # 2. La duree du titre sans condition de demarrage.
        (WIDGETS, "if client.etat == crate::gui::client::Etat::Demarrage) {\n        dessine_duree_chargement(",
         "if true) {\n        dessine_duree_chargement("),
        # 3. Le bureau salit la zone de la jauge a chaque changement.
        (WM, "if jauge_change && demarrage {", "if jauge_change {"),
        # 4. La jauge dite visible hors demarrage.
        (WM, "jauge_visible |= demarrage && client.jauge.visible();", "jauge_visible |= client.jauge.visible();"),
    ]
    for n, (f, a, b) in enumerate(negatifs, 1):
        if not mutation(f, a, b):
            print(f"chargement unique : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"CHARGEMENT_UNIQUE_OK negatifs={len(negatifs)} niveau=statique")
    return 0


if __name__ == "__main__":
    sys.exit(main())
