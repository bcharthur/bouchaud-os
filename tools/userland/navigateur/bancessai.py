"""Banc d'essai de la mise en page : deterministe, hors reseau, comparable.

## Pourquoi celui-ci et pas le tableau de bord

`suivi.sh` mesure ce qui compte — le corpus reel, sites compris — mais ses
temps ne sont pas comparables d'une execution a l'autre : ils incluent le
reseau, et la machine elle-meme derive. Constate en cours de route :
l'analyse HTML, que rien dans un changement d'index CSS ne peut toucher, a
double entre deux executions du tableau de bord. Comparer un « avant » d'il y a
une heure a un « apres » de maintenant, c'est mesurer la machine, pas le code.

Ce banc ne charge que les temoins locaux, ne touche jamais au reseau, et
alterne les mesures pour que la derive s'annule au lieu de s'imputer au
changement. C'est le seul dispositif qui permette de dire « ce commit a rendu
la mise en page plus rapide » sans se tromper.

    python3 bancessai.py              # mesure la version courante
    python3 bancessai.py --json       # de quoi comparer par programme

Ce qu'il rend : le temps de mise en page cumule sur les temoins, le temps de
cascade, et les compteurs de travail (poses de boites, essais de selecteur,
tailles intrinseques calculees ou evitees). Les compteurs comptent plus que les
temps : ils ne derivent pas.
"""
import json
import os
import statistics
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.append(os.path.join(os.path.dirname(os.path.abspath(__file__)),
                             "..", "build-test-moteur"))

import apercu  # noqa: E402
apercu.installe_hote(1280, 900)

from moteur import telemetrie  # noqa: E402
import moteur  # noqa: E402
from moteur import html as moteur_html  # noqa: E402

DOSSIER = os.path.join(os.path.dirname(os.path.abspath(__file__)), "tests", "pages")

# Les temoins qui exercent la mise en page. On ecarte ceux qui dependent du
# temps ou du reseau : leur variance noierait le signal.
TEMOINS = ["article.html", "cards.html", "flex.html", "grid.html",
           "login-form.html", "navbar.html", "responsive.html",
           "sticky-header.html", "table.html", "modal.html", "dropdown.html",
           # Le temoin lourd : 4 000 regles et 1 500 elements, aux proportions
           # d'un vrai site. Sans lui le banc ne peut rien dire d'un index de
           # selecteurs — les autres temoins tiennent en quelques dizaines de
           # regles, et un index n'y a rien a filtrer. Fabrique par
           # `tests/pages/fabrique-lourde.py`.
           "feuille-lourde.html"]

COMPTEURS = ("css.selecteurs_essayes", "css.selecteurs_ecartes",
             "css.consultations", "layout.dispose_bloc", "layout.cascade_element",
             "layout.cascade_evitee", "layout.mesures_texte",
             "layout.mesures_evitees", "layout.etendue_contenu",
             "intrinseque.calculees", "intrinseque.evitees",
             "layout.passes")


def une_passe():
    """Un tour complet sur tous les temoins. Rend (temps, compteurs)."""
    telemetrie.reinitialise()
    telemetrie.active(True)
    for nom in TEMOINS:
        chemin = os.path.join(DOSSIER, nom)
        if not os.path.exists(chemin):
            continue
        with open(chemin, encoding="utf-8") as fichier:
            source = fichier.read()
        doc = moteur.Document(
            moteur.reseau.Reponse("file://" + chemin, source, "text/html", 200),
            1280, journal=lambda n, t: None)
        doc.remet_en_page(1280, 900)
        # Une seconde mise en page a une autre largeur : c'est ce que fait un
        # redimensionnement, et c'est la que les caches de passe se prouvent.
        doc.remet_en_page(980, 900)
    phases = {p["phase"]: p["ms"] for p in telemetrie.temps()}
    compteurs = telemetrie.compteurs()
    return phases, compteurs


def mesure(tours=5):
    layouts = []
    cascades = []
    derniers = {}
    # Un tour a blanc : le premier paie l'import des polices et le rechauffage
    # des caches de module, qui n'ont rien a voir avec ce qu'on mesure.
    une_passe()
    for _ in range(tours):
        phases, compteurs = une_passe()
        layouts.append(phases.get("mise_en_page", 0.0))
        cascades.append(phases.get("css", 0.0))
        derniers = compteurs
    return {
        "layout_ms_median": statistics.median(layouts),
        "layout_ms_min": min(layouts),
        "cascade_ms_median": statistics.median(cascades),
        "tours": tours,
        "compteurs": {cle: derniers.get(cle, 0) for cle in COMPTEURS},
    }


def main():
    tours = 5
    for argument in sys.argv[1:]:
        if argument.startswith("--tours="):
            tours = int(argument.split("=", 1)[1])
    resultat = mesure(tours)
    if "--json" in sys.argv:
        print(json.dumps(resultat))
        return
    print("banc d'essai — %d temoins, %d tours" % (len(TEMOINS), resultat["tours"]))
    print("  mise en page   median %8.1f ms   min %8.1f ms"
          % (resultat["layout_ms_median"], resultat["layout_ms_min"]))
    print("  cascade CSS    median %8.1f ms" % resultat["cascade_ms_median"])
    print()
    for cle, valeur in resultat["compteurs"].items():
        print("  %-28s %10d" % (cle, valeur))


if __name__ == "__main__":
    main()
