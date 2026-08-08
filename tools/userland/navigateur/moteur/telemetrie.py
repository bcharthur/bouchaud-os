"""Ce que la page a demande et que le moteur n'a pas su faire.

## Pourquoi ce module existe

Jusqu'ici, diagnostiquer un site cassé se faisait a l'oeil : on comparait une
capture a ce que Firefox affiche, on devinait la cause, on cherchait dans le
code. Trois pages suffisent a ne plus savoir par quoi commencer, et la
tentation devient d'implementer ce que Chrome a plutot que ce qui manque
vraiment.

Ce module renverse la charge de la preuve. Le moteur note, pendant qu'il
travaille, chaque chose qu'il rencontre et ne sait pas traiter : une propriete
CSS qu'il ignore, un selecteur qu'il ne sait pas compiler, une API Web qu'un
script est alle chercher, une ressource qui n'est pas arrivee, une erreur
JavaScript. A la fin, `texte()` rend un rapport ou chaque manque porte son
nombre d'occurrences.

La priorite ne se discute plus : elle se lit.

## Ce que le rapport classe, et pourquoi

Toutes les proprietes CSS manquantes ne coutent pas la meme chose. En ignorer
une qui arrondit un coin donne une page laide ; en ignorer une qui place un
element donne une page illisible, ou un menu par-dessus le texte. Le rapport
range donc chaque manque selon l'effet visible :

* `BLOQUANT` — la mise en page est fausse : positionnement, dimension,
  debordement, empilement. C'est ce qui rend un site inutilisable.
* `FONCTIONNEL` — quelque chose ne repond plus : un bouton, un champ, une
  interaction, une navigation.
* `TYPOGRAPHIE` — le texte est lisible mais mal compose : cesures, espacement,
  troncature, direction.
* `VISUEL` — la page est juste mais ne ressemble pas a ce qu'elle devrait :
  filtres, fondus, masques.
* `COSMETIQUE` — personne ne le remarquerait sans comparer cote a cote.

Un manque `COSMETIQUE` vu dix mille fois reste moins urgent qu'un `BLOQUANT`
vu trois fois, et le rapport les presente dans cet ordre.

## Cout

Rien n'est enregistre tant que `active()` n'a pas ete appele. Les points
d'instrumentation se reduisent alors a un test de booleen sur un module deja
importe, ce qui est le prix d'un attribut. Le navigateur ordinaire ne paie
donc pas la mesure.
"""

import json
import os
import time

# --- Etat ---------------------------------------------------------------------

ACTIVE = False

# `categorie -> cle -> {"n": occurrences, "exemples": [...]}`
_notes = {}
EXEMPLES_MAX = 3


def active(oui=True):
    """Allume ou eteint la collecte. Eteinte, elle ne coute rien."""
    global ACTIVE
    ACTIVE = bool(oui)
    return ACTIVE


def active_par_environnement():
    """Allume la collecte si `BO_BROWSER_TELEMETRIE` le demande."""
    if os.environ.get("BO_BROWSER_TELEMETRIE", "").strip() not in ("", "0"):
        active(True)
    return ACTIVE


def reinitialise():
    """Vide ce qui a ete note. Une page par mesure, sinon rien n'est comparable."""
    _notes.clear()
    _chronos.clear()


def note(categorie, cle, exemple=None):
    """Enregistre une occurrence. Le point d'entree unique de tout le module."""
    if not ACTIVE:
        return
    seau = _notes.setdefault(categorie, {})
    entree = seau.get(cle)
    if entree is None:
        entree = {"n": 0, "exemples": []}
        seau[cle] = entree
    entree["n"] += 1
    if exemple is not None and len(entree["exemples"]) < EXEMPLES_MAX:
        texte_exemple = str(exemple)
        if len(texte_exemple) > 120:
            texte_exemple = texte_exemple[:117] + "..."
        if texte_exemple not in entree["exemples"]:
            entree["exemples"].append(texte_exemple)


# --- Temps passe --------------------------------------------------------------

_chronos = {}


class chrono:
    """Mesure une phase du moteur. Ne coute rien quand la collecte est eteinte.

    Instrumenter avant d'optimiser : sans ces nombres, « le moteur est lent »
    ne designe aucun code en particulier, et l'optimisation se porte sur ce
    qu'on croit couteux plutot que sur ce qui l'est.
    """

    __slots__ = ("nom", "_depart")

    def __init__(self, nom):
        self.nom = nom
        self._depart = 0.0

    def __enter__(self):
        if ACTIVE:
            self._depart = time.perf_counter()
        return self

    def __exit__(self, *_):
        if not ACTIVE:
            return False
        ecoule = time.perf_counter() - self._depart
        entree = _chronos.get(self.nom)
        if entree is None:
            _chronos[self.nom] = [1, ecoule]
        else:
            entree[0] += 1
            entree[1] += ecoule
        return False


def temps():
    """`phase -> (appels, millisecondes cumulees)`, du plus couteux au moins."""
    ordonne = sorted(_chronos.items(), key=lambda t: -t[1][1])
    return [{"phase": nom, "appels": n, "ms": round(total * 1000.0, 2)}
            for nom, (n, total) in ordonne]


# --- Classement des manques CSS -----------------------------------------------

BLOQUANT = "BLOQUANT"
FONCTIONNEL = "FONCTIONNEL"
TYPOGRAPHIE = "TYPOGRAPHIE"
VISUEL = "VISUEL"
COSMETIQUE = "COSMETIQUE"
INCONNU = "INCONNU"

ORDRE_IMPACT = (BLOQUANT, FONCTIONNEL, TYPOGRAPHIE, VISUEL, COSMETIQUE, INCONNU)

# Les proprietes que le moteur lit reellement. Cette liste n'est pas decorative :
# `test_moteur.py` la confronte au code de `moteur/` pour qu'elle ne puisse pas
# se desynchroniser en silence.
IMPLEMENTEES = frozenset("""
align-content align-items align-self
animation animation-delay animation-direction animation-duration
animation-fill-mode animation-iteration-count animation-name
animation-play-state animation-timing-function
aspect-ratio background background-color background-image
border border-bottom border-bottom-color border-bottom-width
border-color border-left border-left-color border-left-width
border-radius border-right border-right-color border-right-width
border-style border-top border-top-color border-top-width border-width
bottom box-shadow box-sizing
clear color column-gap content display
flex flex-basis flex-direction flex-flow flex-grow flex-shrink flex-wrap
float font font-family font-size font-style font-weight
gap grid-area grid-column grid-column-end grid-column-start
grid-row grid-row-end grid-row-start grid-template grid-template-areas
grid-template-columns grid-template-rows
height justify-content justify-items
left line-height list-style list-style-type
margin margin-bottom margin-left margin-right margin-top
max-height max-width min-height min-width
object-fit opacity order outline overflow overflow-x overflow-y
padding padding-bottom padding-left padding-right padding-top
place-content place-items position right row-gap
src text-align text-decoration top transform
transition transition-delay transition-duration transition-property
transition-timing-function
unicode-range vertical-align visibility white-space width z-index
text-transform
inset inset-block inset-inline inset-block-start inset-block-end
inset-inline-start inset-inline-end
margin-block margin-inline margin-block-start margin-block-end
margin-inline-start margin-inline-end
padding-block padding-inline padding-block-start padding-block-end
padding-inline-start padding-inline-end
border-block-start-width border-block-end-width
border-inline-start-width border-inline-end-width
border-block-start-color border-block-end-color
border-inline-start-color border-inline-end-color
border-block-start-style border-block-end-style
border-inline-start-style border-inline-end-style
block-size inline-size max-block-size max-inline-size
min-block-size min-inline-size
grid-gap grid-row-gap grid-column-gap
-webkit-box-sizing -moz-box-sizing -webkit-border-radius -moz-border-radius
-webkit-box-shadow -moz-box-shadow
-webkit-transform -moz-transform -ms-transform -o-transform
-webkit-transition -moz-transition -webkit-animation -moz-animation
-webkit-flex -webkit-flex-direction -webkit-justify-content
-webkit-align-items -webkit-order
""".split())

# Le reste de la plate-forme, avec l'effet qu'a son absence. Ce qui n'est ni
# ici ni au-dessus ressort en `INCONNU` : un prefixe constructeur, une
# invention de cadre applicatif, ou un manque a classer.
CATALOGUE = {}


def _range(impact, noms):
    for nom in noms.split():
        CATALOGUE[nom] = impact


_range(BLOQUANT, """
inset inset-block inset-inline inset-block-start inset-block-end
inset-inline-start inset-inline-end
margin-block margin-inline margin-block-start margin-block-end
margin-inline-start margin-inline-end
padding-block padding-inline padding-block-start padding-block-end
padding-inline-start padding-inline-end
block-size inline-size max-block-size max-inline-size
min-block-size min-inline-size
grid grid-auto-columns grid-auto-flow grid-auto-rows
columns column-count column-width column-span column-fill
float-reference contain contain-intrinsic-size content-visibility
overflow-anchor overscroll-behavior overscroll-behavior-x overscroll-behavior-y
table-layout border-collapse border-spacing caption-side empty-cells
""")

_range(FONCTIONNEL, """
cursor pointer-events user-select touch-action resize
scroll-behavior scroll-snap-type scroll-snap-align scroll-margin
scroll-padding scroll-padding-top scroll-margin-top
appearance accent-color caret-color
will-change animation-composition
""")

_range(TYPOGRAPHIE, """
letter-spacing word-spacing text-indent text-transform text-overflow
text-wrap text-wrap-mode word-break overflow-wrap word-wrap hyphens
line-clamp -webkit-line-clamp -webkit-box-orient
font-variant font-variant-numeric font-feature-settings font-stretch
font-kerning font-optical-sizing font-variation-settings font-display
text-decoration-line text-decoration-color text-decoration-thickness
text-decoration-style text-underline-offset text-rendering
direction unicode-bidi writing-mode text-orientation
tab-size quotes list-style-position list-style-image
text-shadow vertical-align-last text-align-last
""")

_range(VISUEL, """
filter backdrop-filter mix-blend-mode background-blend-mode
mask mask-image mask-size mask-position clip-path
background-size background-position background-repeat background-attachment
background-clip background-origin
border-image border-image-source border-image-slice
outline-offset outline-color outline-width outline-style
box-decoration-break isolation
transform-origin transform-style perspective perspective-origin backface-visibility
rotate scale translate
""")

_range(COSMETIQUE, """
color-scheme forced-color-adjust print-color-adjust
image-rendering shape-outside shape-margin
counter-reset counter-increment counter-set
scrollbar-width scrollbar-color scrollbar-gutter
all zoom
""")


def impact_propriete(nom):
    """L'effet qu'a l'absence de cette propriete."""
    if nom in IMPLEMENTEES:
        return None
    return CATALOGUE.get(nom, INCONNU)


def propriete_ignoree(nom, valeur=None):
    """Une declaration que le moteur a lue mais ne sait pas honorer."""
    if not ACTIVE:
        return
    if nom.startswith("--"):
        # Les proprietes personnalisees sont supportees : elles n'ont pas de
        # sens propre a implementer.
        return
    impact = impact_propriete(nom)
    if impact is None:
        return
    note("css_propriete", "%s\t%s" % (impact, nom), valeur)


def valeur_rejetee(nom, valeur, raison=""):
    """Une propriete connue dont cette valeur precise n'a pas ete comprise.

    C'est le manque le plus sournois : `display: grid` marche, `display: flow-root`
    ne marche pas, et rien ne le dit. Sans cette note, le rapport declarerait la
    propriete supportee.
    """
    note("css_valeur", "%s: %s" % (nom, valeur), raison or None)


def selecteur_ignore(texte, raison=""):
    note("css_selecteur", texte.strip()[:80], raison or None)


def arobase_ignoree(nom, prelude=""):
    note("css_arobase", nom.lower(), prelude or None)


def api_manquante(nom, detail=None):
    """Une API Web qu'un script est alle chercher et qui n'existe pas."""
    note("api", nom, detail)


def erreur_js(message, source=None):
    note("js_erreur", str(message)[:160], source)


def ressource_echouee(url, code, destination=""):
    note("ressource", "%s %s" % (code, destination or "?"), url)


def balise_inconnue(nom):
    note("html_balise", nom.lower())


# --- Rapport ------------------------------------------------------------------

def rapport():
    """Les notes, triees, sous une forme serialisable."""
    resultat = {}
    for categorie, seau in _notes.items():
        entrees = [{"cle": cle, "n": v["n"], "exemples": v["exemples"]}
                   for cle, v in seau.items()]
        entrees.sort(key=lambda e: (-e["n"], e["cle"]))
        resultat[categorie] = entrees
    return resultat


def resume_css():
    """Les proprietes manquantes regroupees par impact, du plus grave au moins."""
    par_impact = {impact: [] for impact in ORDRE_IMPACT}
    for entree in rapport().get("css_propriete", []):
        impact, _, nom = entree["cle"].partition("\t")
        par_impact.setdefault(impact, []).append((nom, entree["n"]))
    for liste in par_impact.values():
        liste.sort(key=lambda paire: (-paire[1], paire[0]))
    return par_impact


def json_ligne(contexte=None):
    """Une ligne JSON par page mesuree, faite pour etre agregee."""
    charge = {"contexte": contexte or {}, "notes": rapport()}
    return json.dumps(charge, ensure_ascii=False, sort_keys=True)


_TITRES = {
    "api": "APIs Web absentes",
    "css_propriete": "Proprietes CSS non honorees",
    "css_valeur": "Valeurs CSS non comprises",
    "css_selecteur": "Selecteurs non compiles",
    "css_arobase": "Regles @ ignorees",
    "html_balise": "Balises HTML inconnues",
    "js_erreur": "Erreurs JavaScript",
    "ressource": "Ressources non chargees",
}


def texte(titre=None, largeur=None):
    """Le rapport lisible. Vide si rien n'a manque — ce qui est une bonne nouvelle."""
    lignes = []
    if titre:
        lignes.append(titre)
        lignes.append("=" * len(titre))
        lignes.append("")

    par_impact = resume_css()
    total_css = sum(n for liste in par_impact.values() for _, n in liste)
    if total_css:
        lignes.append("Proprietes CSS non honorees — %d declarations" % total_css)
        for impact in ORDRE_IMPACT:
            liste = par_impact.get(impact) or []
            if not liste:
                continue
            vus = sum(n for _, n in liste)
            lignes.append("  %-12s %4d declarations, %d proprietes"
                          % (impact, vus, len(liste)))
            for nom, n in liste[:12]:
                lignes.append("      %5d  %s" % (n, nom))
            if len(liste) > 12:
                lignes.append("      ...    %d autres" % (len(liste) - 12))
        lignes.append("")

    donnees = rapport()
    for categorie in ("api", "css_valeur", "css_selecteur", "css_arobase",
                      "js_erreur", "ressource", "html_balise"):
        entrees = donnees.get(categorie)
        if not entrees:
            continue
        lignes.append("%s — %d" % (_TITRES[categorie],
                                   sum(e["n"] for e in entrees)))
        for entree in entrees[:20]:
            ligne = "  %5d  %s" % (entree["n"], entree["cle"])
            if entree["exemples"]:
                ligne += "   (%s)" % " | ".join(entree["exemples"][:2])
            lignes.append(ligne)
        if len(entrees) > 20:
            lignes.append("  ...    %d autres" % (len(entrees) - 20))
        lignes.append("")

    if not lignes or (titre and len(lignes) == 3):
        lignes.append("Rien a signaler : tout ce que la page a demande a ete traite.")
    return "\n".join(lignes).rstrip() + "\n"
