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
    _compteurs.clear()


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


# --- Compteurs bruts ----------------------------------------------------------

_compteurs = {}


def compte(nom, combien=1):
    """Incremente un compteur. Le prix d'un test de booleen quand c'est eteint.

    Distinct de `note()` : ici il n'y a ni cle secondaire ni exemple, seulement
    un nombre. C'est ce qu'il faut pour compter des dizaines de milliers
    d'evenements par mise en page sans que la mesure coute plus que ce qu'elle
    mesure.
    """
    if not ACTIVE:
        return
    _compteurs[nom] = _compteurs.get(nom, 0) + combien


def compteurs():
    return dict(sorted(_compteurs.items(), key=lambda p: -p[1]))


def temps():
    """`phase -> (appels, millisecondes cumulees)`, du plus couteux au moins."""
    ordonne = sorted(_chronos.items(), key=lambda t: -t[1][1])
    return [{"phase": nom, "appels": n, "ms": round(total * 1000.0, 2)}
            for nom, (n, total) in ordonne]


# --- Categories du diagnostic JavaScript --------------------------------------
#
# Quatre categories, et la frontiere entre les deux premieres est tout l'enjeu.
#
# Un `ReferenceError: X is not defined` ne dit pas si `X` est une API que le
# navigateur devrait fournir ou une variable que la page a oublie de definir.
# Les confondre remplit la feuille de route de noms comme `currentUsr` ou
# `webpackChunk`, qui n'ont d'equivalent dans aucun navigateur et qu'aucune
# implementation ne ferait disparaitre — pendant que les vrais manques se
# perdent dans le bruit.
#
# La regle appliquee : un nom n'est une API navigateur absente que s'il figure
# au registre ci-dessous. Tout le reste est un identifiant applicatif inconnu.
# Se tromper dans ce sens est sans danger : au pire on sous-estime un manque,
# et un manque reel finira par revenir par un autre chemin — un accesseur du
# prelude, une methode absente, une ressource echouee.

API_ABSENTE = "api_absente"
IDENTIFIANT_INCONNU = "identifiant_inconnu"
METHODE_ABSENTE = "methode_absente"
MOIGNON = "moignon_appele"
ERREUR_JS = "erreur_js"
RESSOURCE_ECHOUEE = "ressource"

# Les surfaces de la plate-forme Web, par famille. Un nom present ici et absent
# du moteur est un manque a implementer ; un nom absent d'ici est le probleme de
# la page.
SURFACES_WEB = frozenset("""
window self globalThis document navigator location history screen frames parent top
alert confirm prompt open close print focus blur stop
setTimeout setInterval clearTimeout clearInterval queueMicrotask
requestAnimationFrame cancelAnimationFrame requestIdleCallback cancelIdleCallback
setImmediate reportError structuredClone
fetch Request Response Headers AbortController AbortSignal
XMLHttpRequest XMLHttpRequestUpload FormData URLSearchParams URL
WebSocket EventSource BroadcastChannel MessageChannel MessagePort
Worker SharedWorker ServiceWorker ServiceWorkerContainer
Blob File FileList FileReader FileReaderSync
ReadableStream WritableStream TransformStream ByteLengthQueuingStrategy
TextEncoder TextDecoder TextEncoderStream TextDecoderStream
btoa atob crypto SubtleCrypto Intl
localStorage sessionStorage indexedDB IDBFactory IDBKeyRange IDBDatabase
IDBTransaction IDBObjectStore IDBRequest IDBCursor IDBIndex
caches CacheStorage Cache
Notification PushManager Permissions PermissionStatus
Node Element HTMLElement Document DocumentFragment ShadowRoot Text Comment
Attr NamedNodeMap NodeList HTMLCollection DOMTokenList DOMParser XMLSerializer
Range Selection NodeFilter NodeIterator TreeWalker XPathEvaluator XPathResult
MutationObserver IntersectionObserver ResizeObserver PerformanceObserver
ReportingObserver
Event CustomEvent EventTarget MouseEvent KeyboardEvent PointerEvent TouchEvent
InputEvent FocusEvent WheelEvent DragEvent ClipboardEvent CompositionEvent
SubmitEvent PopStateEvent HashChangeEvent MessageEvent ProgressEvent
StorageEvent ErrorEvent PromiseRejectionEvent CloseEvent BeforeUnloadEvent
AnimationEvent TransitionEvent UIEvent PageTransitionEvent
HTMLFormElement HTMLInputElement HTMLSelectElement HTMLTextAreaElement
HTMLButtonElement HTMLAnchorElement HTMLImageElement HTMLCanvasElement
HTMLScriptElement HTMLLinkElement HTMLStyleElement HTMLTemplateElement
HTMLIFrameElement HTMLMediaElement HTMLVideoElement HTMLAudioElement
HTMLOptionElement HTMLLabelElement HTMLTableElement HTMLDialogElement
HTMLSlotElement HTMLPictureElement HTMLSourceElement HTMLTrackElement
Image Audio Option OffscreenCanvas ImageData ImageBitmap Path2D
CanvasRenderingContext2D CanvasGradient CanvasPattern
WebGLRenderingContext WebGL2RenderingContext
CSS CSSStyleSheet CSSStyleDeclaration CSSRule StyleSheet FontFace FontFaceSet
getComputedStyle matchMedia MediaQueryList getSelection
customElements CustomElementRegistry HTMLUnknownElement
performance Performance PerformanceEntry
MediaSource SourceBuffer MediaRecorder MediaStream RTCPeerConnection
SpeechSynthesis speechSynthesis
AbortError DOMException DOMRect DOMRectReadOnly DOMMatrix DOMPoint
visualViewport scrollX scrollY scrollTo scrollBy scroll pageXOffset pageYOffset
innerWidth innerHeight outerWidth outerHeight devicePixelRatio
addEventListener removeEventListener dispatchEvent postMessage
console isSecureContext origin name closed opener length
trustedTypes navigation scheduler
""".split())

# Prefixes qui trahissent un identifiant d'application ou d'outillage. Ils ne
# peuvent jamais devenir des APIs, meme si un jour l'un d'eux entrait par
# accident dans le registre.
_PREFIXES_APPLICATIFS = ("webpack", "__webpack", "parcel", "vite", "_", "$",
                         "jQuery", "angular", "ng", "React", "Vue", "gtag",
                         "dataLayer", "ga", "fbq", "_paq", "Shopify")


def est_surface_web(nom):
    """Ce nom designe-t-il une API navigateur, ou une variable de la page ?"""
    nom = str(nom or "")
    if not nom:
        return False
    if nom in SURFACES_WEB:
        return True
    for prefixe in _PREFIXES_APPLICATIFS:
        if nom.startswith(prefixe):
            return False
    # Les constructeurs de la plate-forme suivent tous la meme forme : une
    # majuscule, puis un suffixe connu. `HTMLFooElement`, `SVGBarElement`,
    # `FooEvent` sont des surfaces ; `MonComposant` n'en est pas une.
    if nom[:1].isupper() and nom.endswith(("Element", "Event", "Observer",
                                           "Exception", "Error")):
        return nom.endswith(("Element", "Event", "Observer")) or nom in SURFACES_WEB
    return False


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
text-transform pointer-events
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
    note(API_ABSENTE, nom, detail)


def erreur_js(message, source=None):
    note(ERREUR_JS, str(message)[:160], source)


def ressource_echouee(url, code, destination=""):
    """Une sous-ressource perdue, nommee par sa destination.

    La cle est la destination — `script`, `stylesheet`, `image`… — et non
    l'adresse : c'est ainsi qu'un bundle principal en 404 se voit du premier
    coup d'oeil, la ou une cle par URL le noierait parmi les vignettes.
    """
    note(RESSOURCE_ECHOUEE, destination or "?", "%s %s" % (code or 0, url))


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
    API_ABSENTE: "APIs Web absentes",
    IDENTIFIANT_INCONNU: "Identifiants applicatifs non definis",
    METHODE_ABSENTE: "Methodes absentes ou non appelables",
    MOIGNON: "Moignons appeles (API presente mais vide)",
    "css_propriete": "Proprietes CSS non honorees",
    "css_valeur": "Valeurs CSS non comprises",
    "css_selecteur": "Selecteurs non compiles",
    "css_arobase": "Regles @ ignorees",
    "html_balise": "Balises HTML inconnues",
    ERREUR_JS: "Erreurs JavaScript",
    RESSOURCE_ECHOUEE: "Ressources non chargees",
}


_SECTIONS_JS = (
    (API_ABSENTE, "MISSING WEB APIs"),
    (IDENTIFIANT_INCONNU, "UNDEFINED APP IDENTIFIERS"),
    (METHODE_ABSENTE, "UNSUPPORTED / MISSING METHODS"),
    (MOIGNON, "STUBBED APIs USED"),
    (RESSOURCE_ECHOUEE, "RESOURCE FAILURES"),
    (ERREUR_JS, "JAVASCRIPT ERRORS"),
)


def rapport_plateforme(titre="Bouchaud Browser Compatibility Report", lignes_max=15):
    """Le rapport de la plate-forme Web, dans l'ordre ou il se decide.

    Les APIs absentes d'abord parce qu'elles se corrigent en les ecrivant. Les
    identifiants applicatifs ensuite, non pour les implementer — ils n'ont
    d'equivalent dans aucun navigateur — mais parce qu'ils signalent presque
    toujours un bundle qui n'a pas charge, donc une ressource a aller chercher
    plus bas dans le meme rapport.

    Les moignons ont leur propre section, et c'est voulu. Une API presente mais
    vide est souvent pire qu'une absente : la page teste `if (history.pushState)`,
    obtient `true`, prend le chemin moderne, et poursuit avec un navigateur dont
    l'etat ne correspond plus a ce qu'elle croit. Une absence, elle, l'aurait
    fait retomber sur son plan de secours.
    """
    donnees = rapport()
    lignes = ["=== %s ===" % titre]
    vide = True
    for categorie, entete in _SECTIONS_JS:
        entrees = donnees.get(categorie)
        if not entrees:
            continue
        vide = False
        lignes.append("")
        lignes.append(entete)
        for entree in entrees[:lignes_max]:
            lignes.append("  %-40s %6d" % (entree["cle"][:40], entree["n"]))
            for exemple in entree["exemples"][:1]:
                lignes.append("      %s" % exemple[:90])
        if len(entrees) > lignes_max:
            lignes.append("  %-40s %6d" % ("… autres", len(entrees) - lignes_max))

    par_impact = resume_css()
    if any(par_impact.values()):
        vide = False
        lignes.append("")
        lignes.append("CSS UNSUPPORTED")
        for impact in ORDRE_IMPACT:
            liste = par_impact.get(impact) or []
            if not liste:
                continue
            lignes.append("  %-12s %4d declarations, %d proprietes"
                          % (impact, sum(n for _, n in liste), len(liste)))
            for nom, n in liste[:6]:
                lignes.append("      %-34s %6d" % (nom, n))
    if vide:
        lignes.append("")
        lignes.append("  Rien a signaler : tout ce que la page a demande a ete traite.")
    return "\n".join(lignes) + "\n"


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
    for categorie in (API_ABSENTE, IDENTIFIANT_INCONNU, METHODE_ABSENTE,
                      MOIGNON, RESSOURCE_ECHOUEE, ERREUR_JS,
                      "css_valeur", "css_selecteur", "css_arobase",
                      "html_balise"):
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
