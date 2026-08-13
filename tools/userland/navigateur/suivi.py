#!/usr/bin/env python3
"""Tableau de bord du navigateur : tout ce qui se verifie, et son evolution.

    ./tools/userland/suivi.sh              # tout, reseau compris
    ./tools/userland/suivi.sh --rapide     # sans reseau ni Qt, quelques secondes
    ./tools/userland/suivi.sh --histoire   # l'historique, sans rien relancer
    ./tools/userland/suivi.sh --strict     # sort en erreur si quelque chose recule

## Pourquoi cet outil

Le depot savait deja se verifier, mais en trois commandes qui ne se parlaient
pas : `test-moteur.sh` comptait des assertions, `verifie-hote.sh` comptait des
pixels, `compatibilite.py` comptait des manques. Aucune ne disait si le
navigateur allait **mieux qu'hier**, et c'est pourtant la seule question qui
compte quand on travaille sur la duree.

Ce programme les lance toutes, range leurs nombres au meme endroit, et les
compare a la derniere execution. Chaque ligne porte donc trois choses : la
valeur, l'ecart, et le sens de cet ecart — parce que « 306 declarations
ignorees » ne veut rien dire tout seul, alors que « 306, soit 64 de moins »
veut dire quelque chose.

## L'historique

Une ligne JSON par execution dans `tests/suivi.jsonl`, versionnee avec le code.
C'est volontaire : la trajectoire du navigateur fait partie du depot au meme
titre que le navigateur. Un `git log -p tests/suivi.jsonl` raconte l'histoire
de la compatibilite mieux qu'un journal ecrit a la main, parce qu'il ne peut
pas mentir.

## Le sens des ecarts

Toutes les mesures ne s'ameliorent pas dans la meme direction. Le nombre de
declarations CSS ignorees doit baisser ; le nombre d'ecouteurs qu'une page
parvient a poser doit monter. Chaque mesure declare donc son sens, et le
tableau affiche « mieux » ou « pire » plutot que de laisser le lecteur deviner.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time

RACINE = os.path.dirname(os.path.abspath(__file__))
USERLAND = os.path.dirname(RACINE)
DEPOT = os.path.dirname(os.path.dirname(USERLAND))
HISTOIRE = os.path.join(RACINE, "tests", "suivi.jsonl")

# `RESULTAT : 0 verification(s) en echec (731 passees)` ou
# `RESULTAT : 3 verification(s) en echec sur 731`
_BILAN = re.compile(
    r"RESULTAT\s*:\s*(\d+) verification\(s\) en echec (?:\((\d+) passees\)|sur (\d+))")


# --- Les mesures suivies ------------------------------------------------------
#
# `(cle, libelle, sens)`. `sens` vaut "haut" quand monter est un progres,
# "bas" quand descendre en est un. Un `None` signale une mesure sans direction
# — un simple compte de decor.

BAS, HAUT, NEUTRE = "bas", "haut", None

MESURES = [
    ("__section__", "Verifications", None),
    ("moteur.passees", "assertions du moteur", HAUT),
    ("moteur.echecs", "  dont en echec", BAS),
    ("hote.passees", "controles de pixels (Qt reel)", HAUT),
    ("hote.echecs", "  dont en echec", BAS),

    ("__section__", "Compatibilite CSS", None),
    ("css.declarations_ignorees", "declarations ignorees", BAS),
    ("css.proprietes_bloquantes", "  dont proprietes BLOQUANTES", BAS),
    ("css.proprietes_fonctionnelles", "  dont proprietes FONCTIONNELLES", BAS),
    ("css.selecteurs_ignores", "selecteurs non compiles", BAS),
    ("css.valeurs_rejetees", "valeurs non comprises", BAS),
    ("css.arobases_ignorees", "regles @ ignorees", BAS),

    ("__section__", "JavaScript et interactivite", None),
    ("js.erreurs", "erreurs JavaScript", BAS),
    ("js.apis_manquantes", "APIs Web absentes demandees", BAS),
    ("js.identifiants_inconnus", "identifiants applicatifs non definis", BAS),
    ("js.methodes_absentes", "methodes absentes ou non appelables", BAS),
    ("js.moignons", "moignons appeles (API presente mais vide)", BAS),
    ("js.appels_moteur", "appels du pont Python/JS", BAS),
    ("interactions.ecouteurs", "ecouteurs poses par les pages", HAUT),

    ("__section__", "Rendu et ressources", None),
    ("page.boites_effondrees", "boites avec du texte et zero surface", BAS),
    ("page.boites", "boites posees", NEUTRE),
    ("images.echouees", "images non chargees", BAS),
    ("polices.refusees", "polices refusees", BAS),
    ("reseau.ressources_echouees", "ressources non chargees", BAS),

    ("__section__", "Temps (cumul du corpus mesure)", None),
    ("temps.mise_en_page_ms", "mise en page", BAS),
    ("temps.css_ms", "cascade et analyse CSS", BAS),
    ("temps.javascript_ms", "JavaScript", BAS),
    ("temps.html_ms", "analyse HTML", BAS),
    ("temps.peinture_ms", "peinture", BAS),
]

SENS = {cle: sens for cle, _, sens in MESURES if cle != "__section__"}

# Ce qui, en reculant, doit faire echouer `--strict`. Le temps n'y figure pas :
# il depend de la machine et du reseau, et ferait echouer une execution pour
# une raison qui n'a rien a voir avec le code.
SURVEILLEES = (
    "moteur.echecs", "hote.echecs",
    "css.declarations_ignorees", "css.proprietes_bloquantes",
    "css.selecteurs_ignores", "css.valeurs_rejetees",
    "js.erreurs", "page.boites_effondrees", "polices.refusees",
    "js.moignons",
)


# --- Jalons fonctionnels ------------------------------------------------------
#
# Les mesures ci-dessus disent si le moteur travaille moins et se trompe moins.
# Elles ne disent pas s'il sait faire tourner une application — et c'est
# pourtant la seule question qui interesse quelqu'un qui veut s'en servir.
#
# Un jalon est binaire et nomme une capacite entiere. « 1 141 assertions »
# n'apprend rien a personne ; « INDEXEDDB_PERSISTENCE : PASS » dit qu'une page
# peut fermer et rouvrir sans perdre ses donnees.
#
# L'ordre est celui du parcours d'une application : afficher, saisir, naviguer,
# demander, dialoguer, retenir, et tout cela ensemble.
JALONS = [
    ("STATIC_PAGE", "une page s'affiche"),
    ("LOGIN_FORM", "un formulaire se remplit au clavier"),
    ("SPA_NAVIGATION", "la navigation sans rechargement"),
    ("ASYNC_FETCH", "une requete asynchrone met le DOM a jour"),
    ("WEBSOCKET_CHAT", "un WebSocket recoit ce que le serveur pousse"),
    ("INDEXEDDB_PERSISTENCE", "les donnees survivent au rechargement"),
    ("WEBAPP_COMBINED", "le parcours complet, d'un bout a l'autre"),
    ("IFRAME_SAME_ORIGIN", "un cadre de meme origine s'ouvre au parent"),
    ("IFRAME_CROSS_ORIGIN", "un cadre d'une autre origine reste ferme"),
    ("POSTMESSAGE", "les contextes se parlent sans se lire"),
    ("CORS", "une reponse tierce n'est lue qu'avec l'accord du serveur"),
    ("CANVAS_ORIGIN_CLEAN", "une toile contaminee ne se relit pas"),
    ("WSS", "un WebSocket chiffre, avec verification du certificat"),
    ("WEBSOCKET_SUBPROTOCOL", "le sous-protocole se negocie"),
    ("WEB_WORKER_BASIC", "un Worker vit dans son propre monde"),
    ("WEB_WORKER_FETCH", "il fait du reseau par la pile du document"),
    ("WEB_WORKER_INDEXEDDB", "il partage la base de son origine"),
    ("WEB_WORKER_WEBSOCKET", "il ouvre une connexion permanente"),
    ("WEB_WORKER_MESSAGING", "les messages traversent, copies et dans l'ordre"),
    ("WEB_WORKER_LIFECYCLE", "terminate() rend tout, meme sur une boucle"),
    ("STRUCTURED_CLONE", "ce qui ne se clone pas leve, au lieu de disparaitre"),
    ("MESSAGE_CHANNEL", "deux ports se parlent par la file de taches"),
    ("RENDERER_BASIC", "une page vit dans un autre processus"),
    ("RENDERER_CRASH_ISOLATION", "un renderer tue ne fait pas tomber la fenetre"),
    ("RENDERER_MEMORY_ISOLATION", "sa memoire est bornee, celle du chrome non"),
]

CHEMIN_JALONS = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                             "tests", "jalons.json")


def lit_jalons():
    """Ce que la derniere execution des verifications a etabli.

    Rend un dictionnaire vide si le fichier n'existe pas : les jalons
    apparaissent alors comme non mesures, ce qui est la verite — et non comme
    des echecs, ce qui serait un mensonge.
    """
    try:
        with open(CHEMIN_JALONS, "r", encoding="utf-8") as fichier:
            donnees = json.load(fichier)
        return donnees if isinstance(donnees, dict) else {}
    except (OSError, ValueError):
        return {}


# --- Couleurs -----------------------------------------------------------------

class Style:
    def __init__(self, actif):
        self.actif = actif

    def __call__(self, texte, code):
        return "\033[%sm%s\033[0m" % (code, texte) if self.actif else texte

    def gras(self, t):
        return self(t, "1")

    def vert(self, t):
        return self(t, "32")

    def rouge(self, t):
        return self(t, "31")

    def jaune(self, t):
        return self(t, "33")

    def gris(self, t):
        return self(t, "90")

    def cyan(self, t):
        return self(t, "36")


# --- Collecte -----------------------------------------------------------------

def _lance(commande, cwd, silencieux=True):
    """Execute et rend `(code, sortie)`. Ne leve jamais."""
    try:
        fini = subprocess.run(commande, cwd=cwd, capture_output=True,
                              text=True, timeout=1800)
    except FileNotFoundError as e:
        return 127, str(e)
    except subprocess.TimeoutExpired:
        return 124, "delai depasse"
    sortie = (fini.stdout or "") + (fini.stderr or "")
    if not silencieux:
        print(sortie)
    return fini.returncode, sortie


def _bilan(sortie):
    """Extrait `(passees, echecs)` d'une sortie de verification. `None` si absente."""
    trouve = _BILAN.search(sortie)
    if not trouve:
        return None
    echecs = int(trouve.group(1))
    total = int(trouve.group(2) or trouve.group(3) or 0)
    # La forme « en echec (N passees) » donne les reussites ; la forme
    # « en echec sur N » donne le total. On rend toujours des reussites.
    passees = total if trouve.group(2) else max(0, total - echecs)
    return passees, echecs


def verifie_moteur(bavard):
    code, sortie = _lance(["./test-moteur.sh"], USERLAND, silencieux=not bavard)
    bilan = _bilan(sortie)
    if bilan is None:
        return {"etat": "indisponible", "detail": sortie.strip().splitlines()[-3:]}
    passees, echecs = bilan
    return {"etat": "ok" if code == 0 else "echec",
            "passees": passees, "echecs": echecs,
            "lignes_echec": [l.strip(" -") for l in sortie.splitlines()
                             if l.startswith("  - ")]}


def verifie_hote(bavard):
    code, sortie = _lance(["./verifie-hote.sh"], USERLAND, silencieux=not bavard)
    bilan = _bilan(sortie)
    if bilan is None:
        # Qt de developpement absent : ce n'est pas un echec du navigateur, et
        # le confondre avec un echec ferait fuir la lecture du tableau.
        manque = "introuvable" if "introuvable" in sortie else "compilation"
        return {"etat": "indisponible", "raison": manque}
    passees, echecs = bilan
    return {"etat": "ok" if code == 0 else "echec",
            "passees": passees, "echecs": echecs}


def mesure_compatibilite(corpus, bavard):
    """Lance `compatibilite.py` et rend ses observations, page par page."""
    fichier = os.path.join(RACINE, ".suivi-observations.jsonl")
    commande = [sys.executable, "compatibilite.py", "--fixtures", "--json", fichier]
    if corpus:
        commande.insert(2, "--corpus")
    code, sortie = _lance(commande, RACINE, silencieux=not bavard)
    pages = []
    if os.path.exists(fichier):
        with open(fichier, encoding="utf-8") as f:
            for ligne in f:
                ligne = ligne.strip()
                if ligne:
                    pages.append(json.loads(ligne))
        os.remove(fichier)
    if not pages:
        return {"etat": "indisponible", "detail": sortie.strip().splitlines()[-3:]}
    return {"etat": "ok", "pages": pages}


def _somme(pages, axe, cle):
    total = 0
    for page in pages:
        valeur = (page["observations"].get(axe) or {}).get(cle)
        if isinstance(valeur, (int, float)):
            total += valeur
    return total


def _somme_notes(pages, categorie, prefixe=None):
    total = 0
    for page in pages:
        for entree in page["notes"].get(categorie, []):
            if prefixe is None or entree["cle"].startswith(prefixe):
                total += entree["n"]
    return total


def _temps(pages, phase):
    total = 0.0
    for page in pages:
        brut = (page["observations"].get("TEMPS") or {}).get(phase)
        if not brut:
            continue
        try:
            total += float(str(brut).split("ms")[0])
        except ValueError:
            pass
    return round(total, 1)


def assemble(moteur, hote, compat):
    """Range toutes les mesures dans un dictionnaire plat."""
    valeurs = {}
    if moteur.get("etat") != "indisponible":
        valeurs["moteur.passees"] = moteur.get("passees", 0)
        valeurs["moteur.echecs"] = moteur.get("echecs", 0)
    if hote.get("etat") != "indisponible":
        valeurs["hote.passees"] = hote.get("passees", 0)
        valeurs["hote.echecs"] = hote.get("echecs", 0)

    pages = compat.get("pages") or []
    if pages:
        valeurs.update({
            "css.declarations_ignorees": _somme(pages, "CSS", "declarations_ignorees"),
            "css.proprietes_bloquantes": _somme_notes(pages, "css_propriete", "BLOQUANT"),
            "css.proprietes_fonctionnelles":
                _somme_notes(pages, "css_propriete", "FONCTIONNEL"),
            "css.selecteurs_ignores": _somme(pages, "CSS", "selecteurs_ignores"),
            "css.valeurs_rejetees": _somme(pages, "CSS", "valeurs_rejetees"),
            "css.arobases_ignorees": _somme(pages, "CSS", "arobases_ignorees"),
            "js.erreurs": _somme(pages, "JAVASCRIPT", "erreurs"),
            "js.identifiants_inconnus": _somme(pages, "JAVASCRIPT",
                                               "identifiants_inconnus"),
            "js.methodes_absentes": _somme(pages, "JAVASCRIPT", "methodes_absentes"),
            "js.moignons": _somme(pages, "JAVASCRIPT", "moignons_appeles"),
            "js.apis_manquantes": _somme(pages, "JAVASCRIPT", "apis_manquantes"),
            "js.appels_moteur": _somme(pages, "JAVASCRIPT", "appels_moteur"),
            "interactions.ecouteurs": _somme(pages, "INTERACTIONS", "ecouteurs_poses"),
            "page.boites_effondrees": _somme(pages, "MISE_EN_PAGE", "boites_effondrees"),
            "page.boites": _somme(pages, "MISE_EN_PAGE", "boites"),
            "images.echouees": _somme(pages, "IMAGES", "echouees"),
            "polices.refusees": _somme(pages, "POLICES", "polices_refusees"),
            "reseau.ressources_echouees": _somme(pages, "RESEAU", "ressources_echouees"),
            "temps.mise_en_page_ms": _temps(pages, "mise_en_page"),
            "temps.css_ms": _temps(pages, "css"),
            "temps.javascript_ms": _temps(pages, "javascript"),
            "temps.html_ms": _temps(pages, "html"),
            "temps.peinture_ms": _temps(pages, "peinture"),
        })
    return valeurs


# --- Historique ---------------------------------------------------------------

def revision():
    code, sortie = _lance(["git", "rev-parse", "--short", "HEAD"], DEPOT)
    return sortie.strip() if code == 0 else "?"


def lit_histoire():
    if not os.path.exists(HISTOIRE):
        return []
    entrees = []
    with open(HISTOIRE, encoding="utf-8") as f:
        for ligne in f:
            ligne = ligne.strip()
            if ligne:
                try:
                    entrees.append(json.loads(ligne))
                except ValueError:
                    continue
    return entrees


def ecrit_histoire(entree):
    os.makedirs(os.path.dirname(HISTOIRE), exist_ok=True)
    with open(HISTOIRE, "a", encoding="utf-8") as f:
        f.write(json.dumps(entree, ensure_ascii=False, sort_keys=True) + "\n")


def precedente(histoire, portee):
    """La derniere execution de meme portee. Comparer autre chose n'a pas de sens.

    Une execution `--rapide` ne mesure que les temoins locaux ; la comparer a
    une execution complete ferait apparaitre une amelioration spectaculaire qui
    ne serait que l'absence des sites reels.
    """
    for entree in reversed(histoire):
        if entree.get("portee") == portee:
            return entree
    return None


# --- Affichage ----------------------------------------------------------------

def _nombre(valeur):
    if isinstance(valeur, float):
        return "%.0f" % valeur if valeur >= 100 else "%.1f" % valeur
    return "{:,}".format(valeur).replace(",", " ")


def _ecart(cle, courant, ancien, style):
    if ancien is None or cle not in ancien:
        return style.gris("nouveau"), None
    delta = courant - ancien[cle]
    if abs(delta) < 1e-9:
        return style.gris("="), None
    signe = "+" if delta > 0 else ""
    texte = "%s%s" % (signe, _nombre(delta))
    sens = SENS.get(cle)
    if sens is None:
        return style.gris(texte), None
    mieux = (delta > 0) if sens == HAUT else (delta < 0)
    if mieux:
        return style.vert(texte), "mieux"
    return style.rouge(texte), "pire"


def imprime(valeurs, ancien, contexte, style, reculs):
    titre = "SUIVI DU NAVIGATEUR BOUCHAUD"
    entete = "%s  —  %s  %s  (%s)" % (
        titre, contexte["portee"], contexte["date"], contexte["revision"])
    barre = "═" * min(78, len(entete) + 2)
    print()
    print(style.cyan(barre))
    print(" " + style.gras(entete))
    print(style.cyan(barre))

    if ancien:
        print(style.gris("  compare a %s (%s)"
                         % (ancien["date"], ancien.get("revision", "?"))))
    else:
        print(style.gris("  premiere execution de cette portee : rien a comparer"))

    imprime_jalons(style)

    for cle, libelle, _ in MESURES:
        if cle == "__section__":
            print()
            print("  " + style.gras(libelle))
            continue
        if cle not in valeurs:
            continue
        courant = valeurs[cle]
        ecart, jugement = _ecart(cle, courant, ancien and ancien.get("mesures"), style)
        marque = ""
        if jugement == "mieux":
            marque = style.vert("mieux")
        elif jugement == "pire":
            marque = style.rouge("pire")
        print("    %-38s %12s   %10s  %s"
              % (libelle, _nombre(courant), ecart, marque))

    if reculs:
        print()
        print("  " + style.rouge("Reculs sur des mesures surveillees :"))
        for cle, avant, apres in reculs:
            print("    %-40s %s -> %s" % (cle, _nombre(avant), _nombre(apres)))


def imprime_jalons(style):
    """Les capacites, PASS ou FAIL, avant les compteurs.

    En tete du tableau parce que c'est ce qu'on lit en premier quand on veut
    savoir ou en est le navigateur. Un compteur qui s'ameliore pendant qu'un
    jalon tombe est une regression, quoi qu'en dise le compteur.
    """
    jalons = lit_jalons()
    print()
    print("  " + style.gras("Jalons fonctionnels"))
    if not jalons:
        print("    " + style.gris("non mesures — lancer ./tools/userland/test-moteur.sh"))
        return
    for cle, libelle in JALONS:
        if cle not in jalons:
            etat = style.gris("  ??  ")
        elif jalons[cle]:
            etat = style.vert(" PASS ")
        else:
            etat = style.rouge(" FAIL ")
        print("    %-26s %s  %s" % (cle, etat, style.gris(libelle)))
    # Un jalon inconnu du catalogue vaut mieux affiche qu'ignore : c'est
    # probablement un jalon neuf qu'on a oublie d'inscrire ici.
    connus = {cle for cle, _ in JALONS}
    for cle in sorted(set(jalons) - connus):
        etat = style.vert(" PASS ") if jalons[cle] else style.rouge(" FAIL ")
        print("    %-26s %s  %s" % (cle, etat, style.gris("(hors catalogue)")))


def imprime_indisponibles(moteur, hote, compat, style):
    manquants = []
    if moteur.get("etat") == "indisponible":
        manquants.append(("moteur", "test-moteur.sh n'a rien rendu — "
                                    "lancer ./tools/userland/build-quickjs.sh"))
    if hote.get("etat") == "indisponible":
        manquants.append(("hote Qt", "apt install qtbase5-dev python3-dev "
                                     "libbrotli-dev, puis ./tools/userland/verifie-hote.sh"))
    if compat.get("etat") == "indisponible":
        manquants.append(("compatibilite", "Pillow absent, ou aucune page mesurable"))
    if not manquants:
        return
    print()
    print("  " + style.jaune("Non mesure cette fois :"))
    for nom, remede in manquants:
        print("    %-16s %s" % (nom, style.gris(remede)))


def imprime_echecs(moteur, style):
    lignes = moteur.get("lignes_echec") or []
    if not lignes:
        return
    print()
    print("  " + style.rouge("Verifications en echec :"))
    for ligne in lignes[:15]:
        print("    - %s" % ligne[:100])
    if len(lignes) > 15:
        print("    … %d autres" % (len(lignes) - 15))


def imprime_histoire(histoire, style):
    if not histoire:
        print("Aucun historique : lancez ./tools/userland/suivi.sh une fois.")
        return
    print()
    print(" " + style.gras("HISTORIQUE — %d execution(s)" % len(histoire)))
    colonnes = [("moteur.passees", "tests"),
                ("moteur.echecs", "ech."),
                ("css.declarations_ignorees", "css ign."),
                ("css.selecteurs_ignores", "sel."),
                ("js.erreurs", "err. js"),
                ("interactions.ecouteurs", "ecout.")]
    print(style.gris("  %-17s %-9s %-8s " % ("date", "revision", "portee")
                     + " ".join("%8s" % titre for _, titre in colonnes)))
    for entree in histoire[-20:]:
        mesures = entree.get("mesures", {})
        cellules = []
        for cle, _ in colonnes:
            valeur = mesures.get(cle)
            cellules.append("%8s" % ("—" if valeur is None else _nombre(valeur)))
        print("  %-17s %-9s %-8s %s"
              % (entree.get("date", "?")[:16], entree.get("revision", "?"),
                 entree.get("portee", "?"), " ".join(cellules)))


# --- Programme ----------------------------------------------------------------

def principal(argv=None):
    analyseur = argparse.ArgumentParser(
        description="Tableau de bord du navigateur, avec suivi dans le temps.")
    analyseur.add_argument("--rapide", action="store_true",
                           help="sans reseau ni Qt : temoins locaux seulement")
    analyseur.add_argument("--corpus", action="store_true",
                           help="force la mesure des sites reels")
    analyseur.add_argument("--sans-corpus", action="store_true",
                           help="temoins locaux seulement, mais garde l'hote Qt")
    analyseur.add_argument("--histoire", action="store_true",
                           help="affiche l'historique sans rien relancer")
    analyseur.add_argument("--strict", action="store_true",
                           help="sort en erreur si une mesure surveillee recule")
    analyseur.add_argument("--sans-histoire", action="store_true",
                           help="n'ajoute rien au journal de suivi")
    analyseur.add_argument("--bavard", action="store_true",
                           help="montre la sortie brute de chaque verification")
    analyseur.add_argument("--sans-couleur", action="store_true")
    arguments = analyseur.parse_args(argv)

    style = Style(sys.stdout.isatty() and not arguments.sans_couleur)
    histoire = lit_histoire()

    if arguments.histoire:
        imprime_histoire(histoire, style)
        return 0

    avec_corpus = arguments.corpus or not (arguments.rapide or arguments.sans_corpus)
    avec_hote = not arguments.rapide
    portee = "complet" if avec_corpus else ("local" if avec_hote else "rapide")

    print(style.gris("  verifications du moteur…"))
    moteur = verifie_moteur(arguments.bavard)
    if avec_hote:
        print(style.gris("  controles de pixels sur Qt reel…"))
        hote = verifie_hote(arguments.bavard)
    else:
        hote = {"etat": "indisponible", "raison": "mode rapide"}
    print(style.gris("  mesure de compatibilite%s…"
                     % (" (temoins + sites reels)" if avec_corpus else " (temoins)")))
    compat = mesure_compatibilite(avec_corpus, arguments.bavard)

    valeurs = assemble(moteur, hote, compat)
    ancien = precedente(histoire, portee)

    reculs = []
    if ancien:
        anciennes = ancien.get("mesures", {})
        for cle in SURVEILLEES:
            if cle not in valeurs or cle not in anciennes:
                continue
            sens = SENS.get(cle)
            delta = valeurs[cle] - anciennes[cle]
            if abs(delta) < 1e-9:
                continue
            pire = (delta < 0) if sens == HAUT else (delta > 0)
            if pire:
                reculs.append((cle, anciennes[cle], valeurs[cle]))

    contexte = {
        "date": time.strftime("%Y-%m-%d %H:%M"),
        "revision": revision(),
        "portee": portee,
    }
    imprime(valeurs, ancien, contexte, style, reculs)
    imprime_echecs(moteur, style)
    imprime_indisponibles(moteur, hote, compat, style)

    if not arguments.sans_histoire and valeurs:
        entree = dict(contexte)
        entree["mesures"] = valeurs
        ecrit_histoire(entree)
        print()
        print(style.gris("  consigne dans %s  (%d execution(s))"
                         % (os.path.relpath(HISTOIRE, DEPOT), len(histoire) + 1)))

    echec_dur = (moteur.get("echecs") or 0) > 0 or (hote.get("echecs") or 0) > 0
    print()
    if echec_dur:
        print("  " + style.rouge("ECHEC — des verifications ne passent pas."))
        return 1
    if reculs and arguments.strict:
        print("  " + style.rouge("ECHEC — recul sur une mesure surveillee (--strict)."))
        return 2
    print("  " + style.vert("Tout passe.")
          + style.gris("  (--histoire pour la trajectoire, --strict pour barrer les reculs)"))
    return 0


if __name__ == "__main__":
    sys.exit(principal())
