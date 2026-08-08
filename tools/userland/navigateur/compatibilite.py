#!/usr/bin/env python3
"""Mesure ce que le moteur sait faire d'une page reelle, axe par axe.

    ./compatibilite.py https://fr.wikipedia.org/wiki/HTML
    ./compatibilite.py --corpus            # tout le corpus de reference
    ./compatibilite.py --fixtures          # les pages temoins locales
    ./compatibilite.py --corpus --json observations.jsonl

## Ce que cet outil remplace

Avant lui, evaluer la compatibilite du navigateur consistait a ouvrir un site,
regarder la capture et se former une opinion. On en sortait avec un
pourcentage, et un pourcentage ne se corrige pas : il ne dit ni ce qui manque,
ni combien de fois, ni ce que ca coute.

Ici, chaque page est chargee avec la telemetrie allumee, puis decrite sur onze
axes. Chaque observation est un nombre qu'on peut reproduire, comparer avant et
apres un correctif, et agreger sur tout le corpus. Ce qui ressort n'est pas
« Wikipedia s'affiche a 70 % » mais « `letter-spacing` est ignore 41 fois sur
6 sites, impact TYPOGRAPHIE » — une phrase sur laquelle on peut decider.

## Les axes

`RESEAU`, `HTML`, `CSS`, `MISE_EN_PAGE`, `POLICES`, `IMAGES`, `JAVASCRIPT`,
`FORMULAIRES`, `NAVIGATION`, `INTERACTIONS`, `STOCKAGE`. Chacun rend des
compteurs, jamais une appreciation.

## Ce que la mesure ne prouve pas

Le rendu. Cet outil dit ce que le moteur a ignore, pas si le resultat ressemble
a la page. Les deux se completent : `apercu.py` montre, celui-ci explique.
"""

import argparse
import json
import os
import sys
import time

RACINE = os.path.dirname(os.path.abspath(__file__))

# Le corpus de reference. Il n'est pas choisi pour flatter le moteur : chaque
# entree apporte une difficulte que les autres n'ont pas, et deux d'entre elles
# sont connues pour ne pas passer.
# Le corpus de sites reels.
#
# Il est court, et il faut dire pourquoi : l'environnement de developpement
# passe par un proxy qui refuse tout hote hors liste. `example.com`,
# Wikipedia, MDN, Stack Overflow, Hacker News et DuckDuckGo repondent
# « CONNECT tunnel failed, 403 » — le navigateur n'y est pour rien, et les
# mesurer donnerait huit fois la meme page vide en croyant mesurer le Web.
#
# Ce qui reste joignable est pypi.org, et c'est une chance : c'est un site
# moderne complet — grille CSS, polices Web, composants personnalises,
# plusieurs centaines de milliers d'octets de feuille. Cinq de ses pages
# couvrent des structures differentes. Le reste de la couverture vient des
# temoins locaux, qui ont l'avantage d'etre reproductibles.
#
# `--tous` tente quand meme les hotes bloques : sur une machine sans proxy
# restrictif, le corpus complet reprend son sens.
CORPUS = [
    ("pypi-accueil", "https://pypi.org/",
     "accueil : formulaire de recherche, grille de statistiques, en-tete colle"),
    ("pypi-projet", "https://pypi.org/project/requests/",
     "fiche de projet : onglets, barre laterale, listes de versions"),
    ("pypi-aide", "https://pypi.org/help/",
     "document long : titres, listes, ancres, table des matieres"),
    ("pypi-sponsors", "https://pypi.org/sponsors/",
     "beaucoup d'images et de logos, mise en page en cartes"),
    ("pythonhosted", "https://files.pythonhosted.org/",
     "page minimale : index de repertoire servi tel quel"),
]

CORPUS_BLOQUE = [
    ("example.com", "https://example.com/",
     "la page la plus simple du Web"),
    ("wikipedia", "https://fr.wikipedia.org/wiki/HTML",
     "document long, tableaux, notes"),
    ("github", "https://github.com/python/cpython",
     "application dense, composants personnalises"),
    ("stackoverflow", "https://stackoverflow.com/questions/tagged/python",
     "listes complexes, plusieurs colonnes"),
    ("hackernews", "https://news.ycombinator.com/",
     "HTML ancien a base de tableaux"),
    ("mdn", "https://developer.mozilla.org/fr/docs/Web/HTML",
     "documentation moderne, theme sombre"),
    ("duckduckgo", "https://duckduckgo.com/html/?q=bouchaud",
     "page de resultats, formulaire de recherche"),
]

FIXTURES = os.path.join(RACINE, "tests", "pages")


# --- Collecte -----------------------------------------------------------------

def _compte_balises(racine):
    """Le nombre de nœuds par balise, dans tout l'arbre."""
    from moteur.html import Element

    comptes = {}
    pile = [racine]
    nœuds = 0
    while pile:
        nœud = pile.pop()
        nœuds += 1
        if isinstance(nœud, Element):
            comptes[nœud.balise] = comptes.get(nœud.balise, 0) + 1
            pile.extend(nœud.enfants)
    return nœuds, comptes


def _parcourt_boites(boite):
    pile = [boite]
    while pile:
        courante = pile.pop()
        yield courante
        pile.extend(getattr(courante, "enfants", ()) or ())


def _boites_effondrees(boite):
    """Les boites qui portent du texte et n'ont aucune surface.

    C'est le symptome le plus fiable d'une mise en page ratee, et il se mesure
    sans reference : un element qui contient des mots et occupe zero pixel ne
    sera jamais correct, quelle que soit la page.
    """
    effondrees = 0
    for courante in _parcourt_boites(boite):
        if courante.largeur > 0.5 and courante.hauteur > 0.5:
            continue
        element = getattr(courante, "element", None)
        texte = ""
        try:
            texte = (element.texte() or "").strip() if element is not None else ""
        except Exception:  # noqa: BLE001
            texte = ""
        if texte:
            effondrees += 1
    return effondrees


def _debordements(boite, largeur):
    """Les boites qui sortent de la fenetre par la droite."""
    sortent = 0
    for courante in _parcourt_boites(boite):
        if courante.x + courante.largeur > largeur + 2:
            sortent += 1
    return sortent


def mesure(cible, largeur=1280, hauteur=900, battements=4):
    """Charge une page et rend ses observations. Ne leve jamais."""
    from moteur import images, reseau, telemetrie
    import moteur

    telemetrie.reinitialise()
    telemetrie.active(True)
    images.vide()
    reseau.oublie_precharges()

    journal = []
    depart = time.perf_counter()
    observations = {"cible": cible}
    try:
        document = moteur.charge(cible, largeur,
                                 journal=lambda n, t: journal.append((n, t)))
    except Exception as e:  # noqa: BLE001
        observations["erreur"] = str(e)
        return observations, telemetrie.rapport()

    chargement = time.perf_counter() - depart
    document.remet_en_page(largeur, hauteur)
    for _ in range(battements):
        document.rafraichis()
    total = time.perf_counter() - depart

    racine = document.racine
    nœuds, balises = _compte_balises(racine)
    affichage = document.liste_affichage(0.0, largeur, hauteur)
    operations = {}
    for operation in affichage:
        nom = operation[0] if operation else "?"
        operations[nom] = operations.get(nom, 0) + 1

    notes = telemetrie.rapport()
    phases = telemetrie.temps()

    def somme(categorie):
        return sum(e["n"] for e in notes.get(categorie, []))

    boite = document.boite

    observations.update({
        "RESEAU": {
            "code": getattr(document, "code", None),
            "erreur": getattr(document, "erreur", None),
            "ressources_echouees": somme("ressource"),
            "refus_securite": somme("securite_refus"),
            "secondes_chargement": round(chargement, 3),
        },
        "HTML": {
            "nœuds": nœuds,
            "elements": sum(balises.values()),
            "balises_distinctes": len(balises),
            "scripts": balises.get("script", 0),
            "feuilles": balises.get("link", 0) + balises.get("style", 0),
            "iframes": balises.get("iframe", 0),
            "svg": balises.get("svg", 0),
            "canvas": balises.get("canvas", 0),
        },
        "CSS": {
            "regles": _compte_regles(document),
            "declarations_ignorees": somme("css_propriete"),
            "proprietes_ignorees": len(notes.get("css_propriete", [])),
            "valeurs_rejetees": somme("css_valeur"),
            "selecteurs_ignores": somme("css_selecteur"),
            "arobases_ignorees": somme("css_arobase"),
        },
        "MISE_EN_PAGE": {
            "boites": sum(1 for _ in _parcourt_boites(boite)) if boite else 0,
            "hauteur_page": round(float(getattr(document, "hauteur", 0)), 1),
            "operations_peintes": len(affichage),
            "textes_peints": operations.get("texte", 0),
            "boites_effondrees": _boites_effondrees(boite) if boite else 0,
            "boites_debordantes": _debordements(boite, largeur) if boite else 0,
            "secondes_total": round(total, 3),
        },
        "TEMPS": {p["phase"]: "%sms/%d" % (p["ms"], p["appels"]) for p in phases},
        "POLICES": {
            "font_face_declares": len(getattr(document, "_polices_declarees", ()) or ()),
            "polices_posees": sum(1 for v in getattr(document, "_polices", {}).values() if v),
            "polices_refusees": sum(1 for v in getattr(document, "_polices", {}).values() if not v),
        },
        "IMAGES": {
            "balises_img": balises.get("img", 0),
            "chargees": _note_de(notes, "image", "chargee"),
            "echouees": _note_de(notes, "image", "echouee"),
            # Peintes dans le premier ecran seulement : la liste d'affichage est
            # elaguee au viewport, ce compte n'est donc pas comparable aux deux
            # precedents et ne dit rien de la reussite du chargement.
            "peintes_a_l_ecran": (operations.get("image", 0)
                                  + operations.get("imagepart", 0)),
        },
        "JAVASCRIPT": {
            "erreurs": somme(telemetrie.ERREUR_JS),
            "erreurs_distinctes": len(notes.get(telemetrie.ERREUR_JS, [])),
            "apis_manquantes": somme(telemetrie.API_ABSENTE),
            "apis_manquantes_distinctes": len(notes.get(telemetrie.API_ABSENTE, [])),
            "identifiants_inconnus": somme(telemetrie.IDENTIFIANT_INCONNU),
            "methodes_absentes": somme(telemetrie.METHODE_ABSENTE),
            "moignons_appeles": somme(telemetrie.MOIGNON),
            "appels_moteur": somme("api_utilisee"),
        },
        "FORMULAIRES": {
            "formulaires": balises.get("form", 0),
            "champs": balises.get("input", 0) + balises.get("textarea", 0),
            "listes": balises.get("select", 0),
            "boutons": balises.get("button", 0),
            "etiquettes": balises.get("label", 0),
        },
        "NAVIGATION": {
            "liens": balises.get("a", 0),
        },
        "INTERACTIONS": _interactions(document),
        "STOCKAGE": {
            "acces_stockage_local": _note_de(notes, "api_utilisee", "stockage"),
            "acces_temoins": (_note_de(notes, "api_utilisee", "temoins")
                              + _note_de(notes, "api_utilisee", "poseTemoin")),
        },
        "journal": [t for niveau, t in journal if niveau in ("error", "warn")][:10],
    })
    return observations, notes


def _interactions(document):
    """Ce que la page a rendu interactif : ecouteurs poses, et pour quoi.

    Le compte vient du prelude, qui l'incremente dans `addEventListener`. Le
    lire ne demande pas de piloter la page — donc pas de scenario, donc pas de
    resultat qui depende de ce qu'on a pense a essayer.
    """
    contexte = getattr(document, "contexte_js", None)
    if contexte is None:
        return {}
    try:
        total = contexte.execute("globalThis.__bo_ecouteurs || 0")
        types = contexte.execute(
            "JSON.stringify(globalThis.__bo_ecouteurs_types || {})")
    except Exception:  # noqa: BLE001
        return {}
    resultat = {"ecouteurs_poses": int(total or 0)}
    try:
        table = json.loads(types or "{}")
        frequents = sorted(table.items(), key=lambda t: -t[1])[:5]
        if frequents:
            resultat["types"] = ",".join("%s:%d" % p for p in frequents)
    except Exception:  # noqa: BLE001
        pass
    return resultat


def _compte_regles(document):
    """Le nombre de regles CSS retenues, quel que soit leur rangement."""
    index = getattr(document, "regles", None)
    if index is None:
        return None
    if isinstance(index, list):
        return len(index)
    total = len(getattr(index, "universelles", ()) or ())
    for attribut in ("par_id", "par_classe", "par_balise"):
        seau = getattr(index, attribut, None) or {}
        total += sum(len(v) for v in seau.values())
    return total


def _note_de(notes, categorie, cle):
    for entree in notes.get(categorie, []):
        if entree["cle"] == cle:
            return entree["n"]
    return 0


# --- Rendu du rapport ---------------------------------------------------------

AXES = ("RESEAU", "HTML", "CSS", "MISE_EN_PAGE", "TEMPS", "POLICES", "IMAGES",
        "JAVASCRIPT", "FORMULAIRES", "NAVIGATION", "INTERACTIONS", "STOCKAGE")


def imprime(observations, notes, detaille=False):
    print("\n" + "=" * 76)
    print(observations["cible"])
    print("=" * 76)
    if "erreur" in observations:
        print("  page non chargee : %s" % observations["erreur"])
        return
    for axe in AXES:
        valeurs = observations.get(axe) or {}
        if not valeurs:
            continue
        parties = ["%s=%s" % (nom, valeur) for nom, valeur in valeurs.items()
                   if valeur not in (0, None)]
        print("  %-13s %s" % (axe, ", ".join(parties) or "rien a signaler"))
    for message in observations.get("journal", []):
        print("      ! %s" % message[:110])
    if detaille:
        from moteur import telemetrie
        print()
        print(telemetrie.texte())


def agrege(toutes):
    """Additionne les notes de plusieurs pages, categorie par categorie."""
    total = {}
    for notes in toutes:
        for categorie, entrees in notes.items():
            seau = total.setdefault(categorie, {})
            for entree in entrees:
                cumul = seau.setdefault(entree["cle"], {"n": 0, "sites": 0})
                cumul["n"] += entree["n"]
                cumul["sites"] += 1
    return total


def imprime_agrege(total, sites):
    from moteur import telemetrie

    print("\n" + "#" * 76)
    print("# Rapport de compatibilite — %d page(s)" % sites)
    print("#" * 76)

    par_impact = {}
    for cle, cumul in total.get("css_propriete", {}).items():
        impact, _, nom = cle.partition("\t")
        par_impact.setdefault(impact, []).append((nom, cumul["n"], cumul["sites"]))

    print("\n## Proprietes CSS non honorees, par gravite")
    for impact in telemetrie.ORDRE_IMPACT:
        liste = sorted(par_impact.get(impact, ()), key=lambda t: (-t[2], -t[1]))
        if not liste:
            continue
        print("\n### %s — %d proprietes, %d declarations, "
              % (impact, len(liste), sum(n for _, n, _ in liste)))
        for nom, n, s in liste[:25]:
            print("    %-34s %5d declarations   %d/%d sites" % (nom, n, s, sites))
        if len(liste) > 25:
            print("    ... %d autres" % (len(liste) - 25))

    from moteur import telemetrie as _t
    for categorie, titre in ((_t.API_ABSENTE, "APIs Web absentes"),
                             (_t.IDENTIFIANT_INCONNU,
                              "Identifiants applicatifs non definis"),
                             (_t.METHODE_ABSENTE, "Methodes absentes"),
                             (_t.MOIGNON, "Moignons appeles"),
                             ("css_valeur", "Valeurs CSS non comprises"),
                             ("css_selecteur", "Selecteurs non compiles"),
                             ("css_arobase", "Regles @ ignorees"),
                             (_t.ERREUR_JS, "Erreurs JavaScript"),
                             ("ressource", "Ressources non chargees"),
                             ("api_utilisee", "APIs du moteur reellement appelees")):
        seau = total.get(categorie)
        if not seau:
            continue
        liste = sorted(seau.items(), key=lambda t: (-t[1]["sites"], -t[1]["n"]))
        print("\n## %s" % titre)
        for cle, cumul in liste[:30]:
            print("    %-46s %6d   %d/%d sites"
                  % (cle[:46], cumul["n"], cumul["sites"], sites))
        if len(liste) > 30:
            print("    ... %d autres" % (len(liste) - 30))


# --- Programme ----------------------------------------------------------------

def cibles_du_corpus():
    return [(nom, url) for nom, url, _ in CORPUS]


def cibles_des_fixtures():
    if not os.path.isdir(FIXTURES):
        return []
    return [(nom, "file://" + os.path.join(FIXTURES, nom))
            for nom in sorted(os.listdir(FIXTURES)) if nom.endswith(".html")]


def principal(argv=None):
    analyseur = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    analyseur.add_argument("cibles", nargs="*", help="URL ou fichier HTML")
    analyseur.add_argument("--corpus", action="store_true",
                           help="mesure le corpus de sites reels joignables")
    analyseur.add_argument("--tous", action="store_true",
                           help="ajoute les hotes que le proxy de developpement bloque")
    analyseur.add_argument("--fixtures", action="store_true",
                           help="mesure les pages temoins locales")
    analyseur.add_argument("--largeur", type=int, default=1280)
    analyseur.add_argument("--hauteur", type=int, default=900)
    analyseur.add_argument("--json", help="ecrit une ligne JSON par page")
    analyseur.add_argument("--detaille", action="store_true",
                           help="ajoute le rapport complet apres chaque page")
    arguments = analyseur.parse_args(argv)

    liste = []
    if arguments.corpus:
        liste += cibles_du_corpus()
    if arguments.tous:
        liste += [(nom, url) for nom, url, _ in CORPUS_BLOQUE]
    if arguments.fixtures:
        liste += cibles_des_fixtures()
    for cible in arguments.cibles:
        if not cible.startswith(("http://", "https://", "file://")):
            cible = "file://" + os.path.abspath(cible)
        liste.append((cible, cible))
    if not liste:
        analyseur.error("rien a mesurer : donnez une cible, --corpus ou --fixtures")

    sys.path.insert(0, RACINE)
    # `bojs` — QuickJS — est compile par `test-moteur.sh` pour le Python de
    # cette machine. Sans lui, la mesure decrirait un navigateur sans
    # JavaScript, ce qui ne ressemble a aucun navigateur.
    atelier = os.path.join(os.path.dirname(RACINE), "build-test-moteur")
    if os.path.exists(os.path.join(atelier, "bojs.so")):
        # En queue de chemin : cet atelier contient aussi une copie de `moteur`,
        # et la mettre devant mesurerait le code d'hier au lieu de celui-ci.
        sys.path.append(atelier)
    else:
        print("bojs.so absent — lancez ./tools/userland/test-moteur.sh une fois "
              "pour mesurer avec JavaScript.", file=sys.stderr)
    import apercu
    apercu.installe_hote(arguments.largeur, arguments.hauteur)
    from moteur import reseau
    apercu.installe_reseau(reseau)

    toutes = []
    sortie = open(arguments.json, "w") if arguments.json else None
    try:
        for nom, cible in liste:
            observations, notes = mesure(cible, arguments.largeur,
                                         arguments.hauteur)
            observations["nom"] = nom
            imprime(observations, notes, arguments.detaille)
            toutes.append(notes)
            if sortie:
                sortie.write(json.dumps({"observations": observations,
                                         "notes": notes},
                                        ensure_ascii=False, sort_keys=True) + "\n")
    finally:
        if sortie:
            sortie.close()

    imprime_agrege(agrege(toutes), len(toutes))
    return 0


if __name__ == "__main__":
    sys.exit(principal())
