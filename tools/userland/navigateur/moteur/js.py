"""Le JavaScript d'une page, branche sur l'arbre du moteur.

Trois etages, dont c'est ici le milieu :

    prelude.js   `document`, `Element`, `setTimeout`, `fetch`… en JavaScript
    js.py        les operations que le prelude demande, sur l'arbre reel
    bojs         QuickJS lui-meme, en C

Le prelude n'a qu'une porte vers l'exterieur — `__bo_appel(operation, …)` — et
elle aboutit a [`Contexte.appel`]. Ajouter une methode au DOM se fait donc a
deux endroits, tous deux en langage de haut niveau, sans jamais recompiler.

## Ce que le JavaScript peut changer

Tout ce qui touche a l'arbre marque le document a refaire (`sale`). Le
navigateur remet en page et repeint au battement suivant : une page qui
construit son contenu dans `DOMContentLoaded` s'affiche donc reellement, ce qui
est le cas le plus repandu du web moderne.

## Ce qu'il ne peut pas

Pas de `eval` sur du code hote, pas d'acces au systeme de fichiers, pas de
sockets : `bojs` n'expose que ce fichier, et ce fichier n'expose que le
document et le reseau HTTP. Une page ne peut donc pas atteindre l'OS, meme si
son script est hostile.
"""

import base64
import collections
import re
import threading
import time
import urllib.parse

import bo
import bojs

from . import (cors, css, html, images, invalidation,
               origine as mod_origine, reseau,
               securite, stockage,
               telemetrie, websocket)

# Types de nœuds, comme dans la norme DOM.
ELEMENT = 1
TEXTE = 3

# Balises dont le contenu ne doit pas etre re-analyse a la serialisation.
_SANS_FERMETURE = html.VIDES


# --- Selecteurs ---------------------------------------------------------------
#
# Le selecteur de `css.py` sert la cascade : il ignore les attributs et les
# pseudo-classes, ce qui est sans consequence sur une feuille de style. Pour
# `querySelector`, ca ne suffit pas — `[data-role="menu"]` doit designer ce
# qu'il designe, sinon un script recupere l'arbre entier et se trompe partout.

def _groupes(selecteur):
    """Les selecteurs d'une liste, analyses par le moteur de la cascade.

    `querySelector` et la cascade partageaient un probleme et deux solutions :
    chacune avait son analyseur, ses combinateurs reconnus et ses
    pseudo-classes ignorees. Un script pouvait donc ne pas trouver un element
    que la feuille de style venait de peindre. Il n'y a plus qu'un moteur —
    celui de [`css`] — et `:nth-child`, `:not()`, `:is()`, `:has()`, `+` et `~`
    valent des deux cotes.
    """
    return css.groupes(selecteur)


# --- Serialisation ------------------------------------------------------------

def _echappe(texte):
    return (texte.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;"))


def serialise(nœud, externe=False):
    """Rend le HTML d'un nœud : son contenu, ou lui-meme avec `externe`."""
    morceaux = []
    if isinstance(nœud, html.Texte):
        return nœud.contenu
    if externe:
        _ouvrante(nœud, morceaux)
    for enfant in nœud.enfants:
        if isinstance(enfant, html.Texte):
            morceaux.append(enfant.contenu if nœud.balise in html.BRUTES
                            else _echappe(enfant.contenu))
        else:
            morceaux.append(serialise(enfant, True))
    if externe and nœud.balise not in _SANS_FERMETURE:
        morceaux.append("</%s>" % nœud.balise)
    return "".join(morceaux)


def _ouvrante(element, morceaux):
    morceaux.append("<" + element.balise)
    for nom, valeur in element.attributs.items():
        morceaux.append(' %s="%s"' % (nom, valeur.replace('"', "&quot;")))
    morceaux.append(">")


# --- Contexte -----------------------------------------------------------------

# `Uncaught ReferenceError: foo is not defined`, sous les formes que QuickJS et
# l'hote produisent.
_NON_DEFINI = re.compile(
    r"(?:ReferenceError:\s*)?['\"`]?([A-Za-z_$][\w$]*)['\"`]?\s+is not defined")

# `x.closest is not a function` — on retient `closest`, le membre reclame. Un
# `not a function` sans nom ne dit rien d'exploitable, et lui en inventer un
# serait pire que d'admettre l'ignorance.
_MEMBRE_NON_FONCTION = re.compile(
    r"[\w$\].]*?\.([A-Za-z_$][\w$]*)\s+is not a function")

# `TypeError: cannot read property 'includes' of undefined` — QuickJS nomme la
# propriete ici, et c'est la forme la plus utile de toutes : c'est exactement
# celle qui tuait le JavaScript de pypi.org. Le message ne dit pas si le trou
# est dans le moteur ou dans la page, mais il nomme ce qu'il faut aller voir,
# et l'exemple conserve garde la phrase entiere pour trancher.
_PROPRIETE_INDEFINIE = re.compile(
    r"cannot read property ['\"`]([^'\"`]+)['\"`] of (?:undefined|null)")

_GENRES_ERREUR = ("TypeError", "RangeError", "SyntaxError", "ReferenceError",
                  "URIError", "EvalError", "InternalError", "AggregateError")


def _genre_erreur(texte):
    """Le nom de l'erreur, pour regrouper. `Error` si rien ne se reconnait."""
    for genre in _GENRES_ERREUR:
        if genre in texte:
            return genre
    return "Error"


class Contexte:
    """Le JavaScript d'un document : son interprete, ses nœuds, ses minuteries."""

    def __init__(self, document, journal=None, budget_ms=5000):
        self.document = document
        self.journal = journal or (lambda niveau, texte: None)
        self._sale = False
        self.navigation = None
        # Vrai quand un `pushState`/`popstate` a change l'adresse sans
        # recharger : le navigateur doit rafraichir sa barre d'adresse sans
        # refaire de requete.
        self.url_changee = False
        # L'historique de session du document, construit paresseusement : une
        # page qui n'appelle jamais `pushState` n'en paie pas le prix.
        self._historique = None
        self._index_historique = 0
        # Manques effectivement rencontres par cette page. Une API simplement
        # absente du moteur n'est pas comptee tant qu'un script ne l'appelle pas.
        self.diagnostics = collections.Counter()

        self._contexte = bojs.cree(budget_ms)
        bojs.pont(self._contexte, self.appel)

        self._identifiants = {}    # id(nœud) -> entier
        self._noeuds = {}          # entier -> nœud
        self._prochain = 1

        self._minuteries = {}      # identifiant -> (echeance, delai, repete)
        self._reponses = []        # (identifiant, reponse) a livrer au battement
        # Les reponses arrivent depuis les fils reseau : la file se partage.
        self._verrou_reponses = threading.Lock()
        self._annulees = set()
        self._fils = None
        # IndexedDB : resultats en attente de livraison, et transactions
        # ouvertes. Les transactions vivent cote Python — la page n'en voit
        # qu'un entier, et ne peut donc pas en fabriquer une.
        self._idb_resultats = []
        self._idb_transactions = {}
        self._prochaine_transaction = 1
        self._idb_fils = None
        # Les `postMessage` recus, en attente du prochain battement.
        self._messages = []
        # Les Workers vivants de ce document, par identifiant. Chacun tient son
        # propre fil et son propre runtime QuickJS ; ce dictionnaire n'est que
        # la laisse qui permet de tous les rendre a la fermeture du document.
        self._workers = {}
        # Les connexions WebSocket ouvertes par cette page, par identifiant.
        self._sockets = {}
        self._lecteurs = {}        # identifiant de nœud -> Lecteur
        self._toiles = {}          # element <canvas> -> operations dessinees
        self._modules = {}         # adresse de module -> source (ou None)
        self._sources_mse = {}     # identifiant MediaSource -> Lecteur
        self._types_mse = {}       # identifiant MediaSource -> type MIME
        self._segments_mse = {}    # segments recus avant rattachement

        # L'ordre compte. `prelude_clone.js` pose `__bo_clone`, dont les deux
        # suivants se servent ; `prelude_partage.js` n'installe rien de
        # lui-meme, il pose `__bo_installe_partage`, que `prelude.js` appelle
        # avec les primitives d'evenement du document. Les deux premiers servent
        # aussi au Worker, avec les siennes.
        for nom in ("prelude_clone.js", "prelude_partage.js", "prelude.js"):
            with open(_chemin_prelude(nom), "r", encoding="utf-8") as f:
                bojs.evalue(self._contexte, f.read(), nom)

    # --- Cycle de vie ---------------------------------------------------------

    def execute_scripts(self):
        """Execute les `<script>` du document, dans l'ordre du texte."""
        for element in list(self.document.racine.parcours()):
            if not isinstance(element, html.Element) or element.balise != "script":
                continue
            type_ = element.attributs.get("type", "").lower()
            if type_ and type_ not in ("text/javascript", "application/javascript",
                                       "module", "text/ecmascript"):
                continue
            source = element.attributs.get("src")
            code = self._telecharge_script(source) if source else element.texte()
            if not code:
                continue
            # Un module a ses propres regles : portee close, `import` resolu au
            # chargement, `this` a `undefined`. L'executer comme un script
            # ordinaire ferait echouer sa premiere ligne.
            module = type_ == "module"
            nom = urllib.parse.urljoin(self.document.url, source) if source \
                else self.document.url + "#script"
            self.execute(code, nom, module=module)

    def execute(self, code, nom="<page>", module=False):
        """Execute du code, en signalant les erreurs plutot qu'en tombant."""
        try:
            with telemetrie.chrono("javascript"):
                return bojs.evalue(self._contexte, code, nom, module)
        except bojs.Erreur as e:
            self._diagnostique_erreur(e, nom)
            self.journal("error", "%s : %s" % (nom, e))
        except Exception as e:  # noqa: BLE001 — une page ne doit pas tuer le navigateur
            self._diagnostique_erreur(e, nom)
            self.journal("error", "%s : %s" % (nom, e))
            self.journal("error", "%s : %s" % (nom, e))
        return None

    def signale_pret(self):
        """Declenche `DOMContentLoaded` puis `load`."""
        self._appelle("__bo_pret", [])

    def tic(self):
        """A appeler regulierement : minuteries echues et reponses en attente."""
        # La file est vidangee d'un coup, sous verrou, puis livree hors verrou :
        # un rappel de page peut emettre une nouvelle requete, et tenir le
        # verrou pendant son execution serait un blocage certain.
        with self._verrou_reponses:
            arrivees, self._reponses = self._reponses, []
            resultats, self._idb_resultats = self._idb_resultats, []
            messages, self._messages = self._messages, []
        for donnees, origine_emetteur, source in messages:
            self._appelle("__bo_message", [donnees, origine_emetteur, source])
        for identifiant, reponse in arrivees:
            self._appelle("__bo_reponse", [identifiant, reponse])
        for identifiant, resultat, erreur in resultats:
            self._appelle("__bo_idb", [identifiant, resultat, erreur])

        maintenant = time.monotonic() * 1000.0
        for identifiant in sorted(self._minuteries):
            entree = self._minuteries.get(identifiant)
            if entree is None or maintenant < entree[0]:
                continue
            echeance, delai, repete = entree
            if repete:
                self._minuteries[identifiant] = (maintenant + delai, delai, True)
            else:
                del self._minuteries[identifiant]
            encore = self._appelle("__bo_minuterie", [identifiant])
            if not encore and repete:
                self._minuteries.pop(identifiant, None)

        self._sockets_battent()
        self._workers_battent()

        # Les lecteurs media avancent au meme rythme : decodage, son pousse
        # vers la carte, image retenue pour la peinture.
        for identifiant, lecteur in list(self._lecteurs.items()):
            try:
                if lecteur.bat():
                    self.sale = True
                if lecteur.termine and not lecteur.__dict__.get("_fin_signalee"):
                    lecteur._fin_signalee = True
                    self._appelle("__bo_media", [identifiant, "ended"])
            except Exception as e:  # noqa: BLE001
                self.journal("warn", "media: %s" % e)

        # Les observateurs de geometrie s'examinent apres la mise en page, pas
        # avant : les boites qu'ils lisent sont celles du dernier passage, et un
        # element cree a l'instant n'en a pas encore.
        if self.document.boite is not None:
            self._appelle("__bo_guette", [])
            self._appelle("__bo_redimensionne", [])

        try:
            bojs.pompe(self._contexte)
        except Exception:
            pass

    def evenement(self, nœud, type_, details=None):
        """Distribue un evenement reel (clic, touche) dans la page."""
        identifiant = None if nœud is None else self._identifiant(nœud)
        return self._appelle("__bo_evenement", [identifiant, type_, details or {}])

    def ferme(self):
        # Les workers d'abord : chacun tient un fil, et un fil qui survit a son
        # document est exactement la fuite qu'on veut ne jamais avoir. Un
        # `terminate()` attend son fil, donc quand cette boucle finit, plus
        # aucun runtime QuickJS de ce document n'existe.
        for travailleur in list(self._workers.values()):
            try:
                travailleur.termine()
            except Exception:  # noqa: BLE001
                pass
        self._workers.clear()
        for connexion in list(self._sockets.values()):
            try:
                connexion.ferme(1001, "page quittee")
            except Exception:  # noqa: BLE001
                pass
        self._sockets.clear()
        if self._fils is not None:
            # Sans attendre : une page qu'on quitte ne doit pas retenir le
            # navigateur le temps qu'une requete lente veuille bien finir.
            self._fils.shutdown(wait=False)
            self._fils = None
        for lecteur in self._lecteurs.values():
            lecteur.ferme()
        self._lecteurs.clear()
        # Le stockage a son propre fil, et ses transactions ouvertes tiennent
        # chacune une copie de leur base. Les oublier ici, c'etait garder en
        # memoire tout ce qu'un iframe avait lu — et un fil de plus par
        # iframe cree.
        if self._idb_fils is not None:
            self._idb_fils.shutdown(wait=False)
            self._idb_fils = None
        # Les `postMessage` recus, en attente du prochain battement.
        self._messages = []
        for transaction in list(self._idb_transactions.values()):
            try:
                transaction.abandonne()
            except Exception:  # noqa: BLE001
                pass
        self._idb_transactions.clear()
        del self._idb_resultats[:]
        del self._reponses[:]
        self._noeuds.clear()
        self._identifiants.clear()
        self._minuteries.clear()
        self._toiles.clear()
        bojs.detruit(self._contexte)
        telemetrie.compte("js.contextes_fermes")

    def _appelle(self, nom, arguments):
        try:
            return bojs.appelle(self._contexte, nom, arguments)
        except bojs.Erreur as e:
            self._diagnostique_erreur(e, nom)
            self.journal("error", str(e))
        except Exception as e:  # noqa: BLE001
            self._diagnostique_erreur(e, nom)
            self.journal("error", str(e))
        return None

    # --- Mesure ---------------------------------------------------------------

    def _op_manque(self, nom):
        """Un script est alle chercher une API que le moteur ne fournit pas.

        Appele depuis les accesseurs poses par le prelude. La valeur rendue
        reste `undefined` cote JavaScript : la mesure n'inflechit pas ce qui
        est mesure.
        """
        self.note(telemetrie.API_ABSENTE, str(nom))
        return None

    def _op_creux(self, nom):
        """Une API que le moteur *fournit* sans la faire.

        Un moignon est souvent pire qu'une absence. La page ecrit
        `if (history.pushState)`, obtient `true`, prend le chemin moderne — et
        poursuit avec un navigateur dont l'etat ne correspond plus a ce qu'elle
        croit. Une absence, elle, l'aurait fait retomber sur son plan de
        secours. C'est pourquoi ces appels-la se comptent a part.
        """
        self.note(telemetrie.MOIGNON, str(nom))
        return None

    def note(self, categorie, cle, exemple=None):
        """Enregistre un manque, pour cette page et pour la mesure globale.

        Deux destinataires, un seul point d'appel. `self.diagnostics` sert le
        rapport de la page — c'est lui que `rapport_compatibilite()` rend et que
        l'apercu imprime. `telemetrie` sert l'agregation sur tout un corpus. Les
        tenir a jour separement les aurait fait diverger au premier oubli.
        """
        self.diagnostics["%s:%s" % (categorie, cle)] += 1
        telemetrie.note(categorie, cle, exemple)

    def echec_ressource(self, destination, url, code, genre=""):
        """Une sous-ressource qui n'est pas arrivee, quelle qu'elle soit.

        Le compteur d'origine ne voyait que `fetch` et `XMLHttpRequest`. Or la
        ressource dont l'echec compte le plus est ailleurs : un `<script src>`
        principal en 404 vide la page de tout comportement, et le rapport
        n'en disait rien. Chaque chargeur passe donc par ici, en nommant sa
        destination — `script`, `module`, `stylesheet`, `image`, `font`,
        `media`, `fetch`, `xhr` — pour qu'un bundle manquant se voie
        immediatement.
        """
        self.diagnostics["ressource_echouee:%s" % destination] += 1
        telemetrie.note(telemetrie.RESSOURCE_ECHOUEE, destination,
                        "%s %s %s" % (code or 0, genre or "", url))

    def _diagnostique_erreur(self, erreur, source=None):
        """Classe une erreur reelle sans lui attribuer un role qu'elle n'a pas.

        La version d'origine rangeait tout `X is not defined` en API navigateur
        absente. C'est faux la plupart du temps : `currentUsr` non defini est un
        defaut de la page, ou la consequence d'un bundle qui n'a pas charge, pas
        une lacune du moteur. Confondre les deux remplirait la feuille de route
        de noms qui n'ont d'equivalent dans aucun navigateur.

        Un nom n'est donc classe en API navigateur que s'il figure au registre
        des surfaces connues. Sinon il est range comme identifiant applicatif
        inconnu : utile a savoir, jamais a implementer.
        """
        texte = str(erreur)
        trouve = _NON_DEFINI.search(texte)
        if trouve:
            nom = trouve.group(1)
            categorie = (telemetrie.API_ABSENTE if telemetrie.est_surface_web(nom)
                         else telemetrie.IDENTIFIANT_INCONNU)
            self.note(categorie, nom, source)
            return
        trouve = _PROPRIETE_INDEFINIE.search(texte)
        if trouve:
            self.note(telemetrie.METHODE_ABSENTE, trouve.group(1), texte[:140])
            return
        trouve = _MEMBRE_NON_FONCTION.search(texte)
        if trouve:
            # `objet.membre is not a function` nomme le membre : exploitable.
            # `not a function` tout court ne nomme rien, et inventer un nom
            # serait pire que d'admettre qu'on ne sait pas lequel.
            self.note(telemetrie.METHODE_ABSENTE, trouve.group(1), source)
            return
        if "not a function" in texte or "is not callable" in texte:
            # QuickJS ne nomme pas le membre pour `x.y()` : le message est un
            # « TypeError: not a function » nu. Lui inventer un nom serait pire
            # que de compter les cas anonymes a part et de garder la trace.
            self.note(telemetrie.METHODE_ABSENTE, "<anonyme>", texte[:120])
            return
        self.note(telemetrie.ERREUR_JS, _genre_erreur(texte), texte[:160])

    def rapport_compatibilite(self):
        """Compteurs observes, stables et directement serialisables."""
        return dict(sorted(self.diagnostics.items()))

    # --- Table des nœuds ------------------------------------------------------

    def _identifiant(self, nœud):
        cle = id(nœud)
        existant = self._identifiants.get(cle)
        if existant is not None:
            return existant
        identifiant = self._prochain
        self._prochain += 1
        self._identifiants[cle] = identifiant
        self._noeuds[identifiant] = nœud
        return identifiant

    def _noeud(self, identifiant):
        if identifiant is None:
            return None
        return self._noeuds.get(int(identifiant))

    def _element(self, identifiant):
        nœud = self._noeud(identifiant)
        return nœud if isinstance(nœud, html.Element) else None

    # --- Le passage unique ----------------------------------------------------

    # `sale` veut dire « le JavaScript a touche a l'arbre ». Toute mutation le
    # leve, et la plupart ne disent pas *ce* qui a change — un `innerHTML`, un
    # nœud insere, un texte remplace. Faute de mieux, elles valent une remise en
    # page complete, et c'est ce que la propriete inscrit.
    #
    # Les rares mutations dont on connait exactement la portee — une propriete
    # de `style`, un attribut — marquent l'invalidation *avant* de lever ce
    # drapeau, et leur marque plus fine est alors ecrasee par celle-ci. C'est
    # voulu pour l'instant : se tromper vers le haut coute du temps, se tromper
    # vers le bas laisse un affichage faux. Les ops precises posent
    # `_sale` directement pour garder leur etendue.
    @property
    def sale(self):
        return self._sale

    @sale.setter
    def sale(self, valeur):
        self._sale = bool(valeur)
        if valeur:
            self.document.invalide.marque(invalidation.MISE_EN_PAGE,
                                          raison="mutation")

    def appel(self, operation, *arguments):
        """Point d'entree de `__bo_appel`, cote Python."""
        if telemetrie.ACTIVE and operation != "manque":
            # Ce que la page utilise vraiment, et pas seulement ce qui lui
            # manque : sans ce compteur, on ne saurait pas si une API implementee
            # sert a dix sites ou a aucun, donc pas ou porter l'effort suivant.
            telemetrie.note("api_utilisee", operation)
        methode = getattr(self, "_op_" + operation, None)
        if methode is None:
            self.journal("warn", "operation JavaScript inconnue : %s" % operation)
            return None
        return methode(*arguments)

    # --- Operations : lecture de l'arbre --------------------------------------

    def _op_console(self, niveau, texte):
        self.journal(niveau, texte)

    def _op_racine(self):
        return self._identifiant(self.document.racine)

    def _op_corps(self):
        corps = self.document.racine.trouve("body")
        return self._identifiant(corps or self.document.racine)

    def _op_tete(self):
        tete = self.document.racine.trouve("head")
        return self._identifiant(tete or self.document.racine)

    def _op_type(self, identifiant):
        return TEXTE if isinstance(self._noeud(identifiant), html.Texte) else ELEMENT

    def _op_balise(self, identifiant):
        element = self._element(identifiant)
        return element.balise if element else ""

    def _op_parent(self, identifiant):
        nœud = self._noeud(identifiant)
        parent = nœud.parent if nœud else None
        return self._identifiant(parent) if parent else None

    def _op_contient(self, identifiant_ancetre, identifiant):
        """L'ancetre contient-il ce nœud ? Un seul passage du pont, pas un par niveau.

        La remontee se faisait cote JavaScript, `parentNode` apres
        `parentNode`, donc un aller-retour par niveau d'arbre. Comme chaque
        `setAttribute` demande a chaque `MutationObserver` a portee d'arbre s'il
        est concerne, et qu'un cadre applicatif en pose un sur la racine, une
        page de pypi.org faisait **900 000 traversees du pont** pour se
        construire. La remontee est la meme ; c'est le nombre d'appels qui
        change.
        """
        ancetre = self._noeud(identifiant_ancetre)
        nœud = self._noeud(identifiant)
        if ancetre is None or nœud is None:
            return False
        courant = nœud
        while courant is not None:
            if courant is ancetre:
                return True
            courant = courant.parent
        return False

    def _op_enfants(self, identifiant, elements_seulement):
        element = self._element(identifiant)
        if not element:
            return []
        enfants = element.enfants
        if elements_seulement:
            enfants = [e for e in enfants if isinstance(e, html.Element)]
        return [self._identifiant(e) for e in enfants]

    def _op_frere(self, identifiant, suivant, element_seulement):
        nœud = self._noeud(identifiant)
        if not nœud or not nœud.parent:
            return None
        fratrie = nœud.parent.enfants
        try:
            position = fratrie.index(nœud)
        except ValueError:
            return None
        pas = 1 if suivant else -1
        position += pas
        while 0 <= position < len(fratrie):
            candidat = fratrie[position]
            if not element_seulement or isinstance(candidat, html.Element):
                return self._identifiant(candidat)
            position += pas
        return None

    def _op_texte(self, identifiant):
        nœud = self._noeud(identifiant)
        if isinstance(nœud, html.Texte):
            return nœud.contenu
        return nœud.texte() if nœud else ""

    def _op_html(self, identifiant, externe=False):
        nœud = self._noeud(identifiant)
        return serialise(nœud, externe) if nœud else ""

    def _op_attribut(self, identifiant, nom):
        element = self._element(identifiant)
        if not element:
            return None
        return element.attributs.get(nom.lower())

    def _op_attributs(self, identifiant):
        element = self._element(identifiant)
        return dict(element.attributs) if element else {}

    def _op_style(self, identifiant, propriete):
        element = self._element(identifiant)
        if not element:
            return None
        for declaration in element.attributs.get("style", "").split(";"):
            if ":" not in declaration:
                continue
            nom, valeur = declaration.split(":", 1)
            if nom.strip().lower() == propriete.lower():
                return valeur.strip()
        return None

    # --- Operations : ecriture -----------------------------------------------

    def _op_poseTexte(self, identifiant, valeur):
        nœud = self._noeud(identifiant)
        if isinstance(nœud, html.Texte):
            nœud.contenu = valeur
        elif isinstance(nœud, html.Element):
            nœud.enfants = []
            nœud.ajoute(html.Texte(valeur))
        self.sale = True

    def _op_poseAttribut(self, identifiant, nom, valeur):
        element = self._element(identifiant)
        if not element:
            return
        # Un attribut peut changer ce que la cascade designe — `class`, `id`,
        # un attribut cible par un selecteur : il faut rejouer le style. Les
        # autres ne touchent que le contenu de l'element.
        self.document.invalide.marque(
            invalidation.STYLE if nom in ("class", "id", "style")
            else invalidation.MISE_EN_PAGE, element, "attribut:%s" % nom)
        self._sale = True
        if valeur is None:
            element.attributs.pop(nom.lower(), None)
        else:
            element.attributs[nom.lower()] = valeur
        self.sale = True

    def _op_poseStyle(self, identifiant, propriete, valeur):
        element = self._element(identifiant)
        if not element:
            return
        declarations = []
        for declaration in element.attributs.get("style", "").split(";"):
            if ":" not in declaration:
                continue
            nom, ancienne = declaration.split(":", 1)
            if nom.strip().lower() != propriete.lower():
                declarations.append((nom.strip(), ancienne.strip()))
        if valeur is not None and valeur != "":
            declarations.append((propriete, valeur))
        element.attributs["style"] = "; ".join(
            "%s: %s" % (n, v) for n, v in declarations)
        # On sait exactement quelle propriete a change : c'est le seul chemin de
        # mutation ou l'etendue se deduit sans deviner. `style.color = "red"`
        # ne demande donc plus une remise en page de tout le document.
        self.document.invalide.marque_propriete(propriete.strip().lower(), element)
        self._sale = True

    def _op_poseHtml(self, identifiant, source):
        element = self._element(identifiant)
        if not element:
            return
        element.enfants = []
        for enfant in html.analyse(source).enfants:
            element.ajoute(enfant)
        self.sale = True

    def _op_insereHtml(self, identifiant, position, source):
        element = self._element(identifiant)
        if not element:
            return
        nouveaux = list(html.analyse(source).enfants)
        position = (position or "").lower()
        if position == "afterbegin":
            for i, enfant in enumerate(nouveaux):
                enfant.parent = element
                element.enfants.insert(i, enfant)
        elif position == "beforeend":
            for enfant in nouveaux:
                element.ajoute(enfant)
        elif position in ("beforebegin", "afterend") and element.parent:
            fratrie = element.parent.enfants
            index = fratrie.index(element) + (1 if position == "afterend" else 0)
            for i, enfant in enumerate(nouveaux):
                enfant.parent = element.parent
                fratrie.insert(index + i, enfant)
        self.sale = True

    def _op_creeElement(self, nom):
        return self._identifiant(html.Element(nom.lower()))

    def _op_creeTexte(self, contenu):
        return self._identifiant(html.Texte(contenu))

    def _op_insere(self, parent_id, enfant_id, avant_id):
        parent = self._element(parent_id)
        enfant = self._noeud(enfant_id)
        if not parent or not enfant:
            return
        # Deplacer, c'est d'abord retirer : sinon le nœud figurerait deux fois.
        if enfant.parent is not None and enfant in enfant.parent.enfants:
            enfant.parent.enfants.remove(enfant)
        enfant.parent = parent
        avant = self._noeud(avant_id)
        if avant is not None and avant in parent.enfants:
            parent.enfants.insert(parent.enfants.index(avant), enfant)
        else:
            parent.enfants.append(enfant)
        self.sale = True

    def _op_retire(self, identifiant):
        nœud = self._noeud(identifiant)
        if nœud and nœud.parent and nœud in nœud.parent.enfants:
            nœud.parent.enfants.remove(nœud)
            nœud.parent = None
            self.sale = True

    def _op_clone(self, identifiant, profond):
        nœud = self._noeud(identifiant)
        if nœud is None:
            return None
        return self._identifiant(_clone(nœud, profond))

    def _op_ecrit(self, source):
        corps = self.document.racine.trouve("body") or self.document.racine
        for enfant in html.analyse(source).enfants:
            corps.ajoute(enfant)
        self.sale = True

    # --- Operations : recherche -----------------------------------------------

    def _racine_de(self, identifiant):
        return self._element(identifiant) or self.document.racine

    def _op_parId(self, valeur):
        for nœud in self.document.racine.parcours():
            if isinstance(nœud, html.Element) and nœud.attributs.get("id") == valeur:
                return self._identifiant(nœud)
        return None

    def _op_select(self, identifiant, selecteur, tous):
        racine = self._racine_de(identifiant)
        groupes = _groupes(selecteur)
        if not groupes:
            return [] if tous else None
        trouves = []
        for nœud in racine.parcours():
            if not isinstance(nœud, html.Element):
                continue
            if css.correspond_un(groupes, nœud):
                if not tous:
                    return self._identifiant(nœud)
                trouves.append(self._identifiant(nœud))
        return trouves if tous else None

    def _op_correspond(self, identifiant, selecteur):
        element = self._element(identifiant)
        if not element:
            return False
        return css.correspond_un(_groupes(selecteur), element)

    def _op_parBalise(self, identifiant, nom):
        racine = self._racine_de(identifiant)
        nom = nom.lower()
        return [self._identifiant(n) for n in racine.parcours()
                if isinstance(n, html.Element) and (nom == "*" or n.balise == nom)]

    def _op_parClasse(self, identifiant, nom):
        racine = self._racine_de(identifiant)
        voulues = set(nom.split())
        return [self._identifiant(n) for n in racine.parcours()
                if isinstance(n, html.Element)
                and voulues <= set(n.attributs.get("class", "").split())]

    def _op_rect(self, identifiant):
        """Position reelle de l'element dans la page, depuis la mise en page."""
        element = self._element(identifiant)
        boite = _boite_de(self.document.boite, element) if element else None
        if boite is None:
            return {"x": 0, "y": 0, "width": 0, "height": 0}
        return {"x": boite.x, "y": boite.y,
                "width": boite.largeur, "height": boite.hauteur}

    def _op_styleCalcule(self, identifiant):
        """Le style resolu d'un element : cascade, heritage et style en ligne.

        C'est ce que `getComputedStyle` doit rendre, et ce qui le distingue de
        `element.style` — lequel ne connait que l'attribut `style`. Une page qui
        lit une couleur de theme ou une hauteur posee par une feuille recevait
        jusqu'ici une chaine vide, et se croyait sur une page sans mise en
        forme.

        La mise en page a deja fait ce calcul : on prend son resultat quand il
        existe. Un element qui n'a pas de boite — parce qu'il vient d'etre cree,
        ou parce qu'il est `display: none` — le fait refaire ici, ce qui est le
        cas rare.
        """
        element = self._element(identifiant)
        if element is None:
            return {}
        boite = _boite_de(self.document.boite, element)
        if boite is not None:
            return dict(boite.style)

        chemin = []
        courant = element
        while courant is not None:
            chemin.append(courant)
            courant = courant.parent
        chemin.reverse()

        # L'heritage se resout de la racine vers l'element : prendre le style du
        # parent seul ne suffirait pas, puisque lui-meme n'est pas encore
        # calcule. C'est le chemin entier qu'on rejoue, ce qui reste peu de
        # travail — un appel isole, sur une lignee de quelques nœuds.
        style = {}
        try:
            for profondeur in range(len(chemin)):
                style = css.applique(self.document.regles, chemin[profondeur],
                                     chemin[:profondeur + 1], style)
        except Exception:  # noqa: BLE001
            return {}
        return dict(style)

    # --- Operations : persistance ---------------------------------------------

    def _op_temoins(self):
        """L'en-tete `Cookie` de la page, tel que `document.cookie` le rend.

        `javascript=True` : les temoins `HttpOnly` restent invisibles ici tout
        en continuant de partir au serveur. C'est exactement ce que l'attribut
        promet, et sans ce drapeau il ne promettait rien.
        """
        return stockage.temoins().pour(self.document.url, javascript=True)

    def _op_poseTemoin(self, declaration):
        """`document.cookie = "nom=valeur; …"` — un seul temoin a la fois."""
        entetes = {"set-cookie": str(declaration)}
        return stockage.temoins().absorbe(self.document.url, entetes,
                                          javascript=True) > 0

    def _op_stockage(self, action, cle=None, valeur=None):
        """`localStorage`, cloisonne par origine et ecrit sur le disque."""
        local = stockage.local()
        url = self.document.url
        if action == "lit":
            return local.lit(url, cle)
        if action == "ecrit":
            local.ecrit(url, cle, valeur)
            return True
        if action == "retire":
            local.retire(url, cle)
            return True
        if action == "cles":
            return local.cles(url)
        if action == "efface":
            local.efface(url)
            return True
        return None

    def _op_moduleAdresse(self, base, nom):
        """Resout une specification d'`import` en adresse absolue.

        QuickJS ne sait pas ce qu'est une URL : c'est ici que `./util.js` vu
        depuis `https://site/app/main.js` devient `https://site/app/util.js`.
        Une specification nue — `import 'lodash'` — n'a pas de sens sans carte
        d'import ; elle est rendue telle quelle, et le chargement echouera en la
        nommant, ce qui est plus parlant qu'une adresse inventee.
        """
        nom = str(nom or "")
        if nom.startswith(("http://", "https://", "data:")):
            return nom
        if not nom.startswith((".", "/")):
            return nom
        depart = str(base or "") or self.document.url
        return urllib.parse.urljoin(depart, nom)

    def _op_moduleSource(self, adresse):
        """Rapporte le texte d'un module. `None` s'il est introuvable.

        Le chargement est synchrone, comme l'exige QuickJS : le graphe entier
        est rapporte avant que le module principal ne s'execute. Chaque adresse
        n'est demandee qu'une fois — un module importe par trois autres n'est
        ni retelecharge ni reevalue.
        """
        adresse = str(adresse or "")
        if adresse in self._modules:
            return self._modules[adresse]
        if not adresse.startswith(("http://", "https://", "file://")):
            self.journal("warn", "module non resoluble : %s" % adresse)
            self._modules[adresse] = None
            return None
        try:
            reponse = reseau.charge(adresse, brut=True,
                                    document=self.document.url,
                                    destination="script")
        except Exception as e:  # noqa: BLE001
            self.echec_ressource("module", adresse, 0, type(e).__name__)
            self.journal("warn", "module %s : %s" % (adresse, e))
            self._modules[adresse] = None
            return None
        if not reponse.code or reponse.code >= 400:
            self.echec_ressource("module", adresse, reponse.code, "http")
            self.journal("warn", "module %s : code %s" % (adresse, reponse.code))
            self._modules[adresse] = None
            return None
        self._modules[adresse] = reponse.contenu
        return reponse.contenu

    def _op_largeurTexte(self, texte, taille, gras=False, fixe=False):
        """Largeur d'un texte dans la fonte reelle, pour `measureText`."""
        try:
            return float(bo.largeur_texte(str(texte), float(taille),
                                          bool(gras), bool(fixe)))
        except Exception:  # noqa: BLE001
            return 0.0

    def _op_toile(self, identifiant, operations):
        """Retient ce qu'un contexte 2D a dessine, pour la prochaine peinture.

        Les operations arrivent dans le vocabulaire de la liste d'affichage —
        rectangles, lignes, textes, images — a ceci pres que les couleurs y sont
        encore des chaines CSS. On les traduit ici, une fois, plutot qu'a chaque
        trame peinte.
        """
        element = self._element(identifiant)
        if element is None:
            return False
        traduites = []
        for operation in operations or []:
            traduite = _operation_toile(operation)
            if traduite is not None:
                traduites.append(traduite)
        self._toiles[element] = traduites
        self.sale = True
        return True

    def toile(self, element):
        """Les operations dessinees sur cette toile, ou `None`."""
        return self._toiles.get(element)

    def _op_toileSouillee(self, operations):
        """Cette suite d'operations rend-elle la toile illisible ?

        La question est tranchee **ici**, cote Python, en relisant les
        operations elles-memes plutot qu'en faisant confiance a un drapeau tenu
        par la page. Un drapeau JavaScript se remet a `false` en une ligne ; ce
        que la toile a reellement dessine, non.

        C'est le vieux manque que l'audit signalait, et il est devenu local le
        jour ou `Origine` a existe : une image d'une autre origine dessinee dans
        une toile doit la contaminer, faute de quoi une page peut relire les
        pixels d'un site tiers ou l'utilisateur est connecte — c'est-a-dire lire
        une page qu'elle n'a pas le droit d'ouvrir.
        """
        mienne = mod_origine.Origine.de_url(self.document.url)
        for operation in operations or []:
            if not operation or operation[0] != "image" or len(operation) < 6:
                continue
            if images.souille(operation[5], mienne):
                telemetrie.compte("toile.souillees")
                self.note(telemetrie.SOP_REFUS, "canvas_tainted",
                          images.origine_de(operation[5]) or "?")
                return True
        return False

    def _op_rasterise(self, operations, largeur, hauteur):
        """Peint des operations de toile hors ecran et rend ses pixels RGBA.

        C'est ce qui donne enfin des pixels a `getImageData` : le contexte 2D
        n'enregistre que des operations, et il faut les jouer pour de vrai avant
        d'en lire un seul. Qt range ses pixels en BGRA ; la norme veut du RGBA,
        d'ou l'echange des deux canaux extremes.
        """
        rasterise = getattr(bo, "rasterise", None)
        if rasterise is None:
            return None
        traduites = []
        for operation in operations or []:
            traduite = _operation_toile(operation)
            if traduite is not None:
                traduites.append(traduite)
        try:
            octets = rasterise(traduites, int(largeur), int(hauteur))
        except Exception as e:  # noqa: BLE001
            self.journal("warn", "rasterise : %s" % e)
            return None
        if not octets:
            return None
        pixels = bytearray(octets)
        pixels[0::4], pixels[2::4] = pixels[2::4], pixels[0::4]
        return list(pixels)

    def _op_imageBrute(self, pixels, largeur, hauteur):
        """Range des pixels RGBA dans le cache d'images et rend leur identifiant.

        Le chemin de retour de `putImageData` : l'hote attend du BGRA, comme
        `rasterise` en rend.
        """
        brute = getattr(bo, "image_brute", None)
        if brute is None:
            return None
        octets = bytearray(int(v) & 0xFF for v in pixels)
        if len(octets) < int(largeur) * int(hauteur) * 4:
            return None
        octets[0::4], octets[2::4] = octets[2::4], octets[0::4]
        try:
            resultat = brute(bytes(octets), int(largeur), int(hauteur))
        except Exception as e:  # noqa: BLE001
            self.journal("warn", "image_brute : %s" % e)
            return None
        return int(resultat[0]) if resultat else None

    def _op_defilement(self):
        """Position de defilement de la page, en pixels."""
        return float(getattr(self.document, "defilement", 0.0) or 0.0)

    # --- Operations : environnement -------------------------------------------

    def _op_titre(self):
        return self.document.titre

    def _op_poseTitre(self, valeur):
        self.document.titre = valeur
        try:
            bo.titre(valeur)
        except Exception:
            pass

    def _op_url(self):
        return self.document.url

    def _op_urlPartie(self, nom):
        morceaux = urllib.parse.urlsplit(self.document.url)
        return {
            "protocol": morceaux.scheme + ":",
            "host": morceaux.netloc,
            "hostname": morceaux.hostname or "",
            "port": str(morceaux.port or ""),
            "pathname": morceaux.path or "/",
            "search": ("?" + morceaux.query) if morceaux.query else "",
            "hash": ("#" + morceaux.fragment) if morceaux.fragment else "",
            "origin": "%s://%s" % (morceaux.scheme, morceaux.netloc),
        }.get(nom, "")

    def _op_navigue(self, url):
        # Le navigateur lit cette demande apres le script : changer de page au
        # milieu d'une execution detruirait l'arbre sous les pieds du moteur.
        self.navigation = urllib.parse.urljoin(self.document.url, url)

    # --- Historique de session -----------------------------------------------
    #
    # `pushState` etait un moignon : la page appelait, rien ne bougeait, et
    # `location.pathname` rendait eternellement l'adresse de depart. Toute
    # application a page unique — c'est-a-dire la forme dominante du Web
    # applicatif — se retrouvait avec une barre d'adresse figee, un bouton
    # « precedent » sans effet, et aucun `popstate`.
    #
    # Ce qui suit tient l'historique **du document** : une liste d'entrees
    # `(adresse, etat, titre)` et un index courant. Il ne remplace pas
    # l'historique du chrome, il le double — et les deux doivent rester
    # d'accord, d'ou `self.navigation` qui reste le canal par lequel une vraie
    # navigation est demandee.

    def _entrees_historique(self):
        if self._historique is None:
            self._historique = [[self.document.url, None, ""]]
            self._index_historique = 0
        return self._historique

    def _op_poseEtat(self, etat, titre, url, remplace):
        """`history.pushState` et `history.replaceState`.

        La difference tient en une ligne : `replaceState` ecrase l'entree
        courante, `pushState` en ajoute une et **tronque tout ce qui suit** —
        c'est ce qui fait qu'apres un retour puis une nouvelle navigation, on
        ne peut plus avancer.

        Aucune requete n'est emise, aucun document n'est recharge : c'est
        exactement ce que la norme demande, et ce que les applications a page
        unique attendent.
        """
        entrees = self._entrees_historique()
        adresse = self.document.url
        if url is not None and str(url) != "":
            adresse = urllib.parse.urljoin(self.document.url, str(url))
        entree = [adresse, etat, str(titre or "")]
        if remplace:
            entrees[self._index_historique] = entree
        else:
            del entrees[self._index_historique + 1:]
            entrees.append(entree)
            self._index_historique = len(entrees) - 1
        # L'adresse du document suit : c'est elle que lisent `location`, la
        # resolution des liens relatifs et la barre d'adresse.
        self.document.url = adresse
        self.url_changee = True
        return adresse

    def _op_historique(self, pas):
        """`history.back()`, `forward()`, `go(n)`.

        Deux cas a ne pas confondre. Si le deplacement reste dans les entrees
        posees par `pushState`, il se resout ici : l'adresse change, `popstate`
        part avec l'etat memorise, et le document n'est pas recharge. S'il sort
        de cette plage — retour vers la page precedente, au sens du chrome —
        c'est une vraie navigation, et elle est deleguee comme avant.
        """
        pas = int(pas)
        entrees = self._entrees_historique()
        cible = self._index_historique + pas
        if pas == 0:
            cible = self._index_historique
        if 0 <= cible < len(entrees):
            self._index_historique = cible
            adresse, etat, _ = entrees[cible]
            self.document.url = adresse
            self.url_changee = True
            self._appelle("__bo_popstate", [etat])
            return True
        # Hors de notre plage : c'est au navigateur de reculer d'une page.
        self.navigation = ("historique", pas)
        return False

    def _op_longueurHistorique(self):
        return len(self._entrees_historique())

    def _op_etatHistorique(self):
        entrees = self._entrees_historique()
        return entrees[self._index_historique][1]

    # --- Edition ---------------------------------------------------------------
    #
    # `input.value` passe par le modele d'edition, pas par l'attribut. C'est ce
    # qui permet a la valeur tapee et a la valeur lue par le script d'etre la
    # meme chose — sans quoi une frappe reelle et un `input.value` se seraient
    # ecrases mutuellement selon l'ordre des operations.

    def _op_valeurChamp(self, identifiant):
        element = self._element(identifiant)
        etat = self.document.edition_de(element) if element is not None else None
        if etat is not None:
            return etat.valeur
        return None

    def _op_poseValeurChamp(self, identifiant, valeur):
        element = self._element(identifiant)
        etat = self.document.edition_de(element) if element is not None else None
        if etat is None:
            return False
        change = etat.remplace_tout(valeur)
        self.document._synchronise_champ(element, etat)
        if change:
            self.sale = True
        return change

    def _op_selectionChamp(self, identifiant):
        element = self._element(identifiant)
        etat = self.document.edition_de(element) if element is not None else None
        if etat is None:
            return None
        debut, fin = etat.selection()
        return {"debut": debut, "fin": fin}

    def _op_poseSelectionChamp(self, identifiant, debut, fin):
        element = self._element(identifiant)
        etat = self.document.edition_de(element) if element is not None else None
        if etat is None:
            return False
        etat.pose_selection(debut, fin)
        self.document.sale_curseur = True
        return True

    # --- WebSocket -------------------------------------------------------------

    def _op_ouvreSocket(self, identifiant, url, protocoles):
        """`new WebSocket(url)`.

        L'ouverture part sur le groupe de fils reseau : la poignee de main est
        un aller-retour, et la faire ici gelerait l'interface le temps qu'elle
        aboutisse — exactement le defaut qu'on a corrige pour `fetch`.
        """
        absolue = urllib.parse.urljoin(self.document.url, str(url))
        absolue = re.sub(r"^http", "ws", absolue, count=1) \
            if absolue.startswith("http") else absolue
        identifiant = int(identifiant)

        def travaille():
            connexion = websocket.Connexion(absolue, protocoles or (),
                                            document=self.document.url)
            self._sockets[identifiant] = connexion
            try:
                connexion.ouvre()
            except websocket.Erreur as e:
                # Un echec d'ouverture se voit comme `error` puis `close`,
                # jamais comme une exception : c'est ce que la page attend.
                connexion._pose("error", str(e))
                connexion._pose("close", {"code": 1006, "raison": str(e),
                                          "propre": False})
                self.echec_ressource("websocket", absolue, 0, str(e))

        self._pool().submit(travaille)
        return True

    def _op_envoieSocket(self, identifiant, donnees):
        connexion = self._sockets.get(int(identifiant))
        return bool(connexion and connexion.envoie(donnees))

    def _op_fermeSocket(self, identifiant, code, raison):
        connexion = self._sockets.get(int(identifiant))
        return bool(connexion and connexion.ferme(int(code or 1000), raison or ""))

    def _op_etatSocket(self, identifiant):
        connexion = self._sockets.get(int(identifiant))
        return websocket.FERME if connexion is None else connexion.etat

    def _sockets_battent(self):
        """Livre au JavaScript ce que les connexions ont recu.

        Appele depuis `tic()`, donc depuis le fil de l'interface : un serveur
        bavard ne peut pas interrompre un script au milieu de son execution.
        """
        for identifiant, connexion in list(self._sockets.items()):
            for type_, charge in connexion.evenements():
                self._appelle("__bo_socket", [identifiant, type_, charge])
                if type_ == "close":
                    self._sockets.pop(identifiant, None)

    def _op_resout(self, adresse):
        """Une adresse relative resolue contre celle du document."""
        return urllib.parse.urljoin(self.document.url, str(adresse or ""))

    # --- Workers --------------------------------------------------------------
    #
    # Un Worker vit sur son propre fil, avec son propre runtime QuickJS. Ce que
    # ce contexte-ci en garde tient en trois choses : la laisse qui permet de le
    # tuer, les services qu'il emprunte, et la file par ou remontent ses
    # messages. Tout le reste est dans [`worker`].

    def _op_wkCree(self, identifiant, url):
        """`new Worker(url)`.

        Le script d'un worker doit etre de **la meme origine** que le document.
        Ce n'est pas une precaution de plus : un worker partage l'origine de son
        createur — donc ses temoins, son stockage, ses bases —, et le charger
        depuis ailleurs reviendrait a executer du code etranger avec les droits
        de la page. La verification est ici, cote Python, et pas dans le
        prelude : une page ne se surveille pas elle-meme.
        """
        from . import worker as mod_worker

        absolue = urllib.parse.urljoin(self.document.url, str(url))
        mienne = mod_origine.Origine.de_url(self.document.url)
        if not mod_origine.Origine.de_url(absolue).meme_origine(mienne):
            self.note(telemetrie.SOP_REFUS, "worker", absolue)
            self.journal("warn",
                         "Worker : %s n'est pas de la meme origine" % absolue)
            return False

        identifiant = int(identifiant)
        self._workers[identifiant] = mod_worker.Worker(
            self, identifiant, absolue, journal=self.journal)
        return True

    def _op_wkPoste(self, identifiant, donnees):
        travailleur = self._workers.get(int(identifiant))
        return bool(travailleur and travailleur.poste(donnees))

    def _op_wkTermine(self, identifiant):
        travailleur = self._workers.pop(int(identifiant), None)
        if travailleur is None:
            return False
        travailleur.termine()
        return True

    def _workers_battent(self):
        """Livre a la page ce que ses workers ont dit.

        Depuis `tic()`, donc depuis le fil de l'interface : un worker bavard ne
        peut pas s'inserer entre deux lignes de la page qui l'ecoute.
        """
        for identifiant, travailleur in list(self._workers.items()):
            for entree in travailleur.recolte():
                self._appelle("__bo_worker_hote",
                              [identifiant, entree[0]] + list(entree[1:]))
            if not travailleur.vivant:
                # `self.close()` depuis le worker : il s'est termine tout seul.
                self._workers.pop(identifiant, None)

    # -- Ce qu'un Worker emprunte a son document --
    #
    # Ces quatre methodes sont la frontiere. Un worker ne parle jamais
    # directement a `reseau` ni a `indexeddb` : il passe par ici, donc par la
    # meme politique et les memes fils que la page. C'est ce qui garantit qu'il
    # n'existe qu'une implementation de Fetch et une seule de CORS.

    def soumets_reseau(self, travail):
        """Fait faire un travail reseau par le groupe de fils du document."""
        self._pool().submit(travail)

    def soumets_stockage(self, travail):
        """Fait faire un travail de stockage par le fil unique du document.

        Le meme fil que la page, et c'est voulu : IndexedDB exige que les
        operations s'executent dans l'ordre ou elles ont ete emises, et un
        second fil rendrait cet ordre indetermine entre un worker et son
        document — qui, eux, partagent la base.
        """
        self._idb_pool().submit(travail)

    def service_idb(self):
        return self._idb_service()

    def recupere_pour(self, methode, url, corps, entetes, avec_temoins):
        """Le `fetch` d'un worker : exactement celui de la page."""
        return self._recupere(methode, url, corps, entetes, avec_temoins)

    def telecharge_worker(self, url):
        """Le source d'un script de worker, ou `None`."""
        return self._telecharge_script(url) or None

    # --- Soumission -----------------------------------------------------------

    def _op_soumets(self, identifiant, avec_evenement):
        """Envoie un formulaire, comme le ferait un clic sur son bouton.

        Deux methodes, deux formes tres differentes, et les confondre casse
        l'une ou l'autre :

        * `GET` place les champs dans la chaine de requete et **remplace** celle
          qui s'y trouvait — un formulaire de recherche soumis deux fois ne doit
          pas accumuler `?q=a&q=b` ;
        * `POST` les met dans le corps, en `application/x-www-form-urlencoded`.

        `requestSubmit()` passe par l'evenement `submit`, donc la page peut
        annuler avec `preventDefault`. `submit()` ne le declenche pas : c'est la
        difference que la norme etablit, et du code s'en sert pour eviter sa
        propre validation.
        """
        formulaire = self._element(identifiant)
        if formulaire is None:
            return False

        if avec_evenement:
            permis = self._appelle("__bo_soumission", [identifiant])
            if permis is False:
                return False

        methode = (formulaire.attributs.get("method") or "get").strip().lower()
        action = (formulaire.attributs.get("action") or "").strip()
        cible = urllib.parse.urljoin(self.document.url, action) if action \
            else self.document.url

        champs = self._appelle("__bo_champsSoumis", [identifiant]) or []
        paires = [(str(nom), str(valeur)) for nom, valeur in champs]
        encode = urllib.parse.urlencode(paires)

        if methode == "post":
            self.navigation = ("soumission", cible, "POST", encode)
        else:
            morceaux = urllib.parse.urlsplit(cible)
            # La chaine de requete est remplacee, pas completee : c'est ce que
            # fait un navigateur, et l'inverse ferait grossir l'adresse a chaque
            # envoi.
            cible = urllib.parse.urlunsplit(
                (morceaux.scheme, morceaux.netloc, morceaux.path, encode, ""))
            self.navigation = cible
        return True

    # --- Foyer ----------------------------------------------------------------

    def _op_foyer(self):
        """`document.activeElement`. `None` quand rien n'est au foyer."""
        actuel = self.document.foyer_actuel()
        return None if actuel is None else self._identifiant(actuel)

    def _op_poseFoyer(self, identifiant):
        """`element.focus()`, et `element.blur()` avec `None`."""
        element = None if identifiant is None else self._element(identifiant)
        if identifiant is not None and element is None:
            return False
        if element is not None and "disabled" in element.attributs:
            # Un champ desactive ne prend pas le foyer, et le lui donner
            # laisserait la page croire qu'on peut y ecrire.
            return False
        return self.document.pose_foyer(element)

    def _op_foyerSuivant(self, sens):
        """`Tab` et `Maj+Tab`, pilotes depuis le JavaScript ou le chrome."""
        atteint = self.document.deplace_foyer(1 if int(sens) >= 0 else -1)
        return None if atteint is None else self._identifiant(atteint)

    def _op_champsFormulaire(self, identifiant):
        """Les controles d'un formulaire, dans l'ordre du document.

        Sert a `form.elements`, que toute soumission asynchrone traverse.
        """
        formulaire = self._element(identifiant)
        if formulaire is None:
            return []
        controles = []
        for nœud in formulaire.parcours():
            if not isinstance(nœud, html.Element):
                continue
            if nœud.balise in ("input", "select", "textarea", "button",
                               "fieldset", "output"):
                controles.append(self._identifiant(nœud))
        return controles

    def _op_formulaireDe(self, identifiant):
        """Le `<form>` qui contient ce controle, ou `None`.

        C'est ce qui fait marcher `input.form` et, surtout, la soumission par
        `Entree` depuis un champ : le navigateur doit savoir quel formulaire
        envoyer.
        """
        element = self._element(identifiant)
        while element is not None:
            if getattr(element, "balise", None) == "form":
                return self._identifiant(element)
            element = getattr(element, "parent", None)
        return None

    def _op_tailleVue(self):
        largeur, hauteur = bo.taille()
        return {"width": largeur, "height": hauteur}

    def _op_base64(self, texte, encode):
        if encode:
            return base64.b64encode(texte.encode("latin-1", "replace")).decode("ascii")
        return base64.b64decode(texte.encode("ascii")).decode("latin-1", "replace")

    # --- Operations : media ---------------------------------------------------
    #
    # Un `<video>` ou un `<audio>` a un lecteur, cree a la premiere demande. Les
    # `MediaSource` en ont un aussi, indexe a part : le JavaScript les cree
    # avant de les rattacher a un element, et parfois sans jamais le faire.

    def _lecteur(self, identifiant, cree=True):
        element = self._element(identifiant)
        if element is None:
            return None
        existant = self._lecteurs.get(identifiant)
        if existant is not None or not cree:
            return existant
        from . import media
        if not media.DISPONIBLE:
            return None
        lecteur = media.Lecteur(journal=self.journal)
        self._lecteurs[identifiant] = lecteur
        return lecteur

    def _op_mediaCharge(self, identifiant):
        """Ouvre ce que designe `src` : une adresse, ou un `MediaSource`."""
        element = self._element(identifiant)
        lecteur = self._lecteur(identifiant)
        if element is None or lecteur is None:
            return False
        source = element.attributs.get("src", "")
        if source.startswith("bo-media:"):
            # Media Source Extensions : c'est la page qui alimentera le flux.
            self._sources_mse[source[len("bo-media:"):]] = lecteur
            return True
        if not source:
            return False
        absolue = urllib.parse.urljoin(self.document.url, source)

        # Une piste audio dissociee, quand la page en declare une : c'est le cas
        # des flux adaptatifs, ou image et son sont deux ressources.
        audio = element.attributs.get("data-audio") or None
        if audio:
            audio = urllib.parse.urljoin(self.document.url, audio)

        # Certains serveurs de media lient l'adresse au client qui l'a
        # demandee — Google le fait pour YouTube — et rendent 403 des que
        # l'en-tete ne correspond plus. La page qui connait ce client le declare
        # ici, et la lecture le reemet a chaque tranche.
        entetes = None
        agent = element.attributs.get("data-agent")
        if agent:
            entetes = {"User-Agent": agent}

        # Le distant se lit par tranches ; le local, d'un bloc.
        if absolue.startswith(("http://", "https://")):
            if not lecteur.ouvre_distant(absolue, audio, entetes):
                return False
        else:
            try:
                reponse = reseau.charge(absolue, brut=True,
                                        document=self.document.url,
                                        destination="media")
            except Exception as e:  # noqa: BLE001
                self.echec_ressource("media", absolue, 0, type(e).__name__)
                self.journal("warn", "media %s : %s" % (absolue, e))
                return False
            if reponse.code and reponse.code >= 400:
                self.journal("warn", "media %s : code %s" % (absolue, reponse.code))
                return False
            if not lecteur.charge(reponse.octets):
                return False
        self._appelle("__bo_media", [identifiant, "loadedmetadata"])
        self._appelle("__bo_media", [identifiant, "canplay"])
        return True

    def _op_mediaJoue(self, identifiant):
        lecteur = self._lecteur(identifiant)
        if lecteur is None:
            return False
        if lecteur.source is None:
            self._op_mediaCharge(identifiant)
        lecteur.joue()
        return True

    def _op_mediaPause(self, identifiant):
        lecteur = self._lecteur(identifiant, cree=False)
        if lecteur is not None:
            lecteur.pause()
        return True

    def _op_mediaCherche(self, identifiant, seconde):
        lecteur = self._lecteur(identifiant, cree=False)
        if lecteur is not None:
            lecteur.cherche(float(seconde))
        return True

    def _op_mediaPosition(self, identifiant):
        lecteur = self._lecteur(identifiant, cree=False)
        return lecteur.position() if lecteur else 0.0

    def _op_mediaDuree(self, identifiant):
        lecteur = self._lecteur(identifiant, cree=False)
        return lecteur.duree() if lecteur else 0.0

    def _op_mediaEnLecture(self, identifiant):
        lecteur = self._lecteur(identifiant, cree=False)
        return bool(lecteur and lecteur.en_lecture)

    def _op_mediaTermine(self, identifiant):
        lecteur = self._lecteur(identifiant, cree=False)
        return bool(lecteur and lecteur.termine)

    def _op_mediaPret(self, identifiant):
        lecteur = self._lecteur(identifiant, cree=False)
        return bool(lecteur and lecteur.source is not None)

    def _op_mediaTampon(self, identifiant):
        lecteur = self._lecteur(identifiant, cree=False)
        return lecteur.duree() if lecteur else 0.0

    def _op_mediaTaille(self, identifiant):
        lecteur = self._lecteur(identifiant, cree=False)
        if lecteur is None:
            return [0, 0]
        return [lecteur.largeur, lecteur.hauteur]

    def _op_mediaVolume(self, identifiant, valeur):
        lecteur = self._lecteur(identifiant, cree=False)
        if lecteur is None:
            return None
        if valeur is None:
            return lecteur.volume
        lecteur.volume = float(valeur)
        return lecteur.volume

    def _op_mediaMuet(self, identifiant, valeur):
        lecteur = self._lecteur(identifiant, cree=False)
        if lecteur is None:
            return None
        if valeur is None:
            return lecteur.muet
        lecteur.muet = bool(valeur)
        return lecteur.muet

    def _op_mediaBoucle(self, identifiant, valeur):
        lecteur = self._lecteur(identifiant, cree=False)
        if lecteur is not None:
            lecteur.boucle = bool(valeur)
        return True

    def image_video(self, nœud):
        """Image courante du lecteur attache a ce nœud, pour la mise en page."""
        identifiant = self._identifiants.get(id(nœud))
        if identifiant is None:
            return None
        lecteur = self._lecteurs.get(identifiant)
        if lecteur is None or lecteur.image is None:
            return None
        return (lecteur.image, lecteur.largeur, lecteur.hauteur)

    def _op_mediaSaitLire(self, type_mime):
        from . import media
        return media.sait_lire(type_mime)

    # --- Operations : Media Source Extensions ---------------------------------

    def _op_mseCree(self, identifiant):
        self._sources_mse.setdefault(str(identifiant), None)
        return True

    def _op_mseType(self, identifiant, type_mime):
        self._types_mse[str(identifiant)] = type_mime
        return True

    def _op_mseAjoute(self, identifiant, octets):
        """`SourceBuffer.appendBuffer` : un segment de plus dans le flux."""
        cle = str(identifiant)
        lecteur = self._sources_mse.get(cle)
        if lecteur is None:
            # L'element ne s'est pas encore rattache : on garde le segment, il
            # sera pousse au rattachement.
            self._segments_mse.setdefault(cle, bytearray())
            self._segments_mse[cle] += bytes(bytearray(octets))
            return len(self._segments_mse[cle])
        donnees = bytes(bytearray(octets))
        en_attente = self._segments_mse.pop(cle, None)
        if en_attente:
            donnees = bytes(en_attente) + donnees
        lecteur.alimente(donnees)
        return len(donnees)

    def _op_mseTampon(self, identifiant):
        lecteur = self._sources_mse.get(str(identifiant))
        return lecteur.duree() if lecteur else 0.0

    def _op_mseFin(self, identifiant):
        return True

    # --- Operations : minuteries et reseau ------------------------------------

    def _op_minuterie(self, identifiant, delai, repete):
        maintenant = time.monotonic() * 1000.0
        self._minuteries[int(identifiant)] = (maintenant + delai, delai, bool(repete))

    def _op_annuleMinuterie(self, identifiant):
        self._minuteries.pop(int(identifiant), None)

    def _op_requete(self, identifiant, methode, url, corps, entetes, synchrone,
                    temoins="same-origin"):
        """`fetch` et `XMLHttpRequest`.

        Le defaut que cette methode corrigeait : la requete rendait bien une
        promesse, mais partait **sur le fil de l'interface**. Le JavaScript
        reprenait la main a l'arrivee de la reponse, pas avant. Mesure sur une
        reponse d'une seconde : zero battement de minuterie, zero evenement,
        zero repeinture pendant tout le vol. La page etait gelee, et la promesse
        ne faisait que masquer le gel.

        Une requete asynchrone part maintenant vers un petit groupe de fils et
        depose sa reponse dans la file que `tic()` vidait deja. Rien d'autre ne
        change : l'ordre de livraison reste « au battement suivant », donc le
        code de retour ne tourne toujours pas avant que `send()` ait rendu la
        main.

        `synchrone` reste synchrone : c'est ce que `XMLHttpRequest` promet dans
        ce mode, et le rendre asynchrone casserait les pages qui s'en servent.
        """
        absolue = urllib.parse.urljoin(self.document.url, url)
        identifiant = int(identifiant)

        # `include` envoie les temoins a toutes les origines ; `same-origin`,
        # la valeur par defaut, ne les envoie qu'a la sienne — auquel cas CORS
        # n'a pas de regle supplementaire a appliquer.
        avec_temoins = str(temoins or "same-origin") == "include"

        if synchrone:
            reponse = self._recupere(methode, absolue, corps, entetes,
                                     avec_temoins)
            self._appelle("__bo_reponse", [identifiant, reponse])
            return

        def travaille():
            reponse = self._recupere(methode, absolue, corps, entetes,
                                     avec_temoins)
            with self._verrou_reponses:
                if identifiant in self._annulees:
                    # Annulee pendant le vol : la reponse est jetee, et le
                    # rappel n'aura jamais lieu — c'est ce qu'attend un
                    # `abort()`.
                    self._annulees.discard(identifiant)
                    return
                self._reponses.append((identifiant, reponse))

        self._pool().submit(travaille)

    def _pool(self):
        """Le groupe de fils reseau, cree au premier besoin.

        Quatre fils : c'est l'ordre de grandeur du budget de connexions par
        hote d'un navigateur, et au-dela le goulot n'est plus le nombre de fils
        mais la reserve de connexions, qui porte deja son propre verrou.

        Cree paresseusement : une page sans requete ne paie aucun fil.
        """
        if self._fils is None:
            from concurrent.futures import ThreadPoolExecutor
            self._fils = ThreadPoolExecutor(max_workers=4,
                                            thread_name_prefix="bo-reseau")
        return self._fils

    # --- Contextes de navigation ----------------------------------------------
    #
    # Toutes les operations `ctx*` prennent un identifiant de BrowsingContext,
    # jamais un objet. La page ne peut donc designer qu'un contexte qu'on lui a
    # donne, et c'est ici — cote Python — que la frontiere d'origine est
    # verifiee. La verifier dans le prelude reviendrait a demander a la page de
    # se surveiller elle-meme.

    def _mon_contexte(self):
        return getattr(self.document, "contexte_navigation", None)

    def _contexte_par_id(self, identifiant):
        """Retrouve un contexte de **cet arbre** par son identifiant.

        La recherche part du sommet de l'arbre courant, pas d'un registre
        global : un onglet ne doit pas pouvoir designer le contexte d'un autre
        onglet en devinant un numero.
        """
        mien = self._mon_contexte()
        if mien is None:
            return None
        if identifiant is None:
            return mien
        try:
            return mien.sommet.par_id(int(identifiant))
        except (TypeError, ValueError):
            return None

    def _op_ctxParent(self, identifiant=None):
        contexte = self._contexte_par_id(identifiant)
        if contexte is None or contexte.parent is None:
            return None
        return contexte.parent.id

    def _op_ctxSommet(self, identifiant=None):
        contexte = self._contexte_par_id(identifiant)
        if contexte is None or contexte.est_sommet:
            return None
        return contexte.sommet.id

    def _op_ctxEnfants(self, identifiant=None):
        contexte = self._contexte_par_id(identifiant)
        return [] if contexte is None else [e.id for e in contexte.enfants]

    def _op_ctxNombreEnfants(self, identifiant=None):
        contexte = self._contexte_par_id(identifiant)
        return 0 if contexte is None else len(contexte.enfants)

    def _op_ctxNom(self, identifiant=None):
        contexte = self._contexte_par_id(identifiant)
        return "" if contexte is None else contexte.nom

    def _op_ctxFerme(self, identifiant=None):
        contexte = self._contexte_par_id(identifiant)
        return True if contexte is None else bool(contexte.ferme)

    def _op_ctxOrigine(self, identifiant=None):
        """`window.origin` : lisible a travers la frontiere, par construction.

        Une origine ne revele rien du contenu — c'est justement ce qui permet a
        deux pages de savoir si elles ont le droit de se parler.
        """
        contexte = self._contexte_par_id(identifiant)
        return "null" if contexte is None else contexte.origine.serialise()

    def _op_ctxUrl(self, identifiant=None):
        """L'adresse d'un contexte, mais seulement en meme origine.

        L'adresse complete d'un document tiers en dit trop : un identifiant de
        session dans le chemin, un jeton dans la requete. La norme la reserve
        au meme origine, et rend l'adresse initiale sinon.
        """
        contexte = self._contexte_par_id(identifiant)
        if contexte is None:
            return "about:blank"
        mien = self._mon_contexte()
        if mien is not None and not mien.meme_origine(contexte):
            telemetrie.compte("sop.url_refusee")
            return "about:blank"
        return contexte.url

    def _op_ctxDocument(self, identifiant=None):
        """`contentDocument` — le point ou la Same-Origin Policy se joue.

        Rendre le document d'une origine differente donnerait a la page hote
        l'acces complet au contenu de la page invitee : son DOM, ses formulaires,
        ses jetons. C'est exactement ce que la SOP existe pour empecher, et c'est
        pourquoi la verification est ici et pas dans le prelude.
        """
        contexte = self._contexte_par_id(identifiant)
        if contexte is None or contexte.ferme:
            return None
        mien = self._mon_contexte()
        if mien is None or not mien.meme_origine(contexte):
            telemetrie.compte("sop.document_refuse")
            self.note(telemetrie.SOP_REFUS, "contentDocument",
                      "%s -> %s" % (mien.origine.serialise() if mien else "?",
                                    contexte.origine.serialise()))
            return None
        document = contexte.document
        if document is None:
            return None
        telemetrie.compte("sop.document_accorde")
        # Pas seulement la racine : `contentDocument` est un **document**, et du
        # code reel lit `.body`, `.title`, `.getElementById(...)`. Rendre le
        # seul nœud racine faisait echouer `doc.body` sur `undefined`, ce qui se
        # lit exactement comme un refus de securite alors que l'acces etait
        # accorde — la pire confusion possible a cet endroit.
        corps = document.racine.trouve("body")
        return {
            "racine": self._identifiant(document.racine),
            "corps": None if corps is None else self._identifiant(corps),
            "titre": document.titre or "",
            "url": document.url,
        }

    def _op_ctxNavigue(self, identifiant, url, remplace=False):
        """Naviguer un autre contexte : permis meme cross-origine.

        C'est voulu, et c'est ce que dit la norme : un parent a le droit de
        changer l'adresse de son iframe sans avoir le droit d'en lire le
        contenu. Naviguer ne revele rien.
        """
        contexte = self._contexte_par_id(identifiant)
        if contexte is None or contexte.ferme:
            return False
        absolue = urllib.parse.urljoin(self.document.url, str(url))
        # Un contexte ne se navigue que depuis son arbre, et un iframe ne peut
        # pas naviguer le sommet d'un autre onglet : `_contexte_par_id` s'en est
        # deja assure.
        if contexte.element is not None:
            contexte.element.pose_attribut("src", absolue)
        return contexte.navigue(absolue, remplace=bool(remplace))

    def _op_ctxFocus(self, identifiant=None):
        contexte = self._contexte_par_id(identifiant)
        if contexte is None:
            return False
        for autre in [contexte.sommet] + contexte.sommet.descendants():
            autre.focalise = autre is contexte
        return True

    def _op_ctxCadre(self, identifiant_element):
        """L'identifiant du contexte porte par un element `<iframe>`."""
        element = self._element(identifiant_element)
        gestionnaire = getattr(self.document, "cadres", None)
        if element is None or gestionnaire is None:
            return None
        contexte = gestionnaire.contexte_de(element)
        return None if contexte is None else contexte.id

    # --- postMessage ----------------------------------------------------------

    def _op_ctxMessage(self, identifiant, donnees, cible_origine):
        """`otherWindow.postMessage(data, targetOrigin)`.

        Le canal legitime entre deux origines : il transporte des donnees, pas
        des references. Une page peut donc dialoguer avec une autre sans jamais
        obtenir son DOM — c'est exactement ce qu'il fallait, et c'est pourquoi
        `postMessage` existe alors que la SOP interdit le reste.

        `targetOrigin` est une **assertion de l'emetteur** : « je n'accepte de
        parler qu'a cette origine-la ». Si le destinataire n'est pas celle-la,
        le message est jete en silence. Ce n'est pas une politesse : sans cette
        verification, une page qui envoie un jeton a son iframe continuerait de
        l'envoyer apres que l'iframe a navigue vers un site tiers.
        """
        destination = self._contexte_par_id(identifiant)
        if destination is None or destination.ferme:
            return False
        emetteur = self._mon_contexte()
        cible = str(cible_origine or "/")

        if cible == "/":
            # `"/"` veut dire « seulement ma propre origine ».
            accorde = emetteur is not None and emetteur.meme_origine(destination)
        elif cible == "*":
            # `"*"` accepte n'importe qui. La norme l'autorise et le deconseille :
            # le message part meme si le destinataire a change d'origine.
            accorde = True
        else:
            attendue = mod_origine.Origine.de_url(cible)
            accorde = (not attendue.est_opaque
                       and attendue.meme_origine(destination.origine))

        if not accorde:
            telemetrie.compte("postmessage.refuses")
            return False

        document = destination.document
        contexte_cible = getattr(document, "contexte_js", None)
        if contexte_cible is None:
            # Le destinataire n'a pas de script : il ne peut rien recevoir. Ce
            # n'est pas une erreur, c'est un message sans auditeur.
            telemetrie.compte("postmessage.sans_auditeur")
            return False

        source = None if emetteur is None else emetteur.id
        origine_emetteur = ("null" if emetteur is None
                            else emetteur.origine.serialise())
        # La livraison passe par la file de la boucle d'evenements du
        # destinataire, jamais par un appel direct : un `postMessage` ne doit
        # pas s'executer au milieu du script de l'emetteur.
        contexte_cible.livre_message(donnees, origine_emetteur, source)
        telemetrie.compte("postmessage.livres")
        return True

    def livre_message(self, donnees, origine_emetteur, source):
        """Range un `message` pour le prochain battement de **ce** contexte."""
        with self._verrou_reponses:
            self._messages.append((donnees, origine_emetteur, source))

    # --- IndexedDB ------------------------------------------------------------
    #
    # Le contrat est le meme que pour le reseau, et pour la meme raison : une
    # ecriture sur disque qui se ferait sur le fil de l'interface figerait la
    # page, et la promesse rendue n'y changerait rien. Chaque appel part vers un
    # fil et depose son resultat dans la file que `tic()` vidange deja — donc
    # aucun rappel de page ne s'execute au milieu d'un script.

    def _idb_service(self):
        from . import indexeddb
        return indexeddb.service(self.document.url)

    def _idb_livre(self, identifiant, resultat=None, erreur=None):
        """Depose un resultat dans la file de la boucle d'evenements."""
        with self._verrou_reponses:
            self._idb_resultats.append((int(identifiant), resultat, erreur))

    def _idb_pool(self):
        """Le fil de stockage. **Un seul**, et c'est le point important.

        La page recoit son jeton de transaction immediatement, mais la
        transaction elle-meme se construit sur le fil. Avec plusieurs fils,
        rien ne garantit que la construction s'execute avant les operations
        qu'on lui adresse : c'est exactement le defaut qu'une epreuve a trouve
        ici — une lecture arrivait avant l'ouverture de sa propre transaction et
        echouait en « transaction inconnue ».

        Un fil unique rend l'ordre de soumission egal a l'ordre d'execution.
        Ce n'est pas un pis-aller : IndexedDB **exige** que les operations d'une
        transaction s'executent dans l'ordre ou la page les a emises, et une
        file unique est la facon la plus simple de ne pas pouvoir se tromper.
        Le stockage reste entierement hors du fil de l'interface, qui est ce
        qu'on voulait.
        """
        if self._idb_fils is None:
            from concurrent.futures import ThreadPoolExecutor
            self._idb_fils = ThreadPoolExecutor(max_workers=1,
                                                thread_name_prefix="bo-idb")
        return self._idb_fils

    def _idb_en_fond(self, identifiant, travail):
        """Fait le travail sur le fil de stockage, et livre ce qu'il rend."""
        from . import indexeddb

        def enrobe():
            try:
                self._idb_livre(identifiant, travail())
            except indexeddb.Erreur as e:
                self._idb_livre(identifiant, None, {"nom": e.nom, "message": str(e)})
            except Exception as e:  # noqa: BLE001 — une page ne tue pas le navigateur
                self._idb_livre(identifiant, None,
                                {"nom": "UnknownError", "message": str(e)})

        self._idb_pool().submit(enrobe)

    def _op_idbOuvre(self, identifiant, base, version):
        """`indexedDB.open(nom, version)`.

        Rend `(version_actuelle, magasins, montee_requise)`. Quand une montee
        est requise, la page recoit `onupgradeneeded` **avant** `onsuccess` et
        n'a le droit de creer des magasins que pendant celui-la : c'est ce qui
        rend le schema d'une base previsible d'un lancement a l'autre.
        """
        version = None if version in (None, "", 0) else int(version)

        def travail():
            actuelle, magasins, montee = self._idb_service().ouvre(str(base), version)
            return {"version": actuelle, "magasins": magasins,
                    "montee": montee,
                    "cible": version if version is not None else actuelle or 1}

        self._idb_en_fond(identifiant, travail)
        return True

    def _op_idbMonte(self, identifiant, base, version, creations, suppressions):
        """Applique ce que `onupgradeneeded` a demande, d'une seule ecriture."""
        listes = [(str(nom), options or {}) for nom, options in (creations or [])]
        retires = [str(nom) for nom in (suppressions or [])]

        def travail():
            magasins = self._idb_service().applique_montee(
                str(base), int(version), listes, retires)
            return {"version": int(version), "magasins": magasins}

        self._idb_en_fond(identifiant, travail)
        return True

    def _op_idbSupprime(self, identifiant, base):
        def travail():
            return self._idb_service().supprime(str(base))

        self._idb_en_fond(identifiant, travail)
        return True

    def _op_idbTransaction(self, identifiant, base, mode):
        """Ouvre une transaction et retient son jeton.

        Le jeton vit cote Python : la page n'en voit qu'un entier, et ne peut
        donc pas fabriquer une transaction qu'elle n'a pas ouverte.
        """
        jeton = self._prochaine_transaction
        self._prochaine_transaction += 1
        mode = "readwrite" if mode == "readwrite" else "readonly"

        def travail():
            self._idb_transactions[jeton] = self._idb_service().transaction(
                str(base), mode)
            return jeton

        self._idb_en_fond(identifiant, travail)
        return jeton

    def _op_idbOperation(self, identifiant, jeton, verbe, magasin, cle, valeur):
        from . import indexeddb

        def travail():
            transaction = self._idb_transactions.get(int(jeton))
            if transaction is None:
                raise indexeddb.Erreur("TransactionInactiveError",
                                       "transaction inconnue")
            return indexeddb.execute(transaction, str(verbe), str(magasin),
                                     cle, valeur)

        self._idb_en_fond(identifiant, travail)
        return True

    def _op_idbTermine(self, identifiant, jeton, valide):
        """Valide ou abandonne une transaction.

        C'est ici que se joue l'atomicite : tant que ce n'est pas appele, rien
        de ce que la transaction a prepare n'est visible d'une autre.
        """
        def travail():
            transaction = self._idb_transactions.pop(int(jeton), None)
            if transaction is None:
                return False
            if valide:
                return transaction.commit()
            transaction.abandonne()
            return False

        self._idb_en_fond(identifiant, travail)
        return True

    def _op_annuleRequete(self, identifiant):
        """`xhr.abort()` et `AbortController.abort()`.

        La requete peut etre deja partie : on ne l'interrompt pas au milieu, on
        s'engage a ne jamais livrer sa reponse. Du point de vue de la page,
        c'est la meme chose — et c'est infiniment plus simple que d'aller
        fermer une prise depuis un autre fil.
        """
        identifiant = int(identifiant)
        with self._verrou_reponses:
            self._annulees.add(identifiant)
            self._reponses[:] = [(i, r) for i, r in self._reponses
                                 if i != identifiant]
        return True

    def _recupere(self, methode, url, corps, entetes, avec_temoins=False):
        """Emet une requete et rend ce que le script a le droit d'en voir.

        Deux decisions distinctes, et les confondre casse tout :

        * **la requete part-elle ?** Presque toujours oui. Une page a toujours
          eu le droit d'envoyer vers une autre origine — c'est ainsi que se
          chargent les images d'un CDN et que poste un formulaire. Le seul cas
          ou l'envoi est retenu est la requete non simple dont le preflight a
          ete refuse, parce qu'elle peut modifier l'etat du serveur ;
        * **la reponse est-elle lisible ?** C'est la que CORS decide, au retour,
          sur `Access-Control-Allow-Origin`.
        """
        mienne = mod_origine.Origine.de_url(self.document.url)

        # Le preflight : le seul endroit ou CORS retient l'envoi.
        if cors.preflight_requis(mienne, url, methode, entetes):
            telemetrie.compte("cors.preflights")
            try:
                sonde = reseau.charge(
                    url, methode="OPTIONS", corps=None,
                    entetes=cors.entetes_preflight(mienne, methode, entetes),
                    brut=True, document=self.document.url, destination="fetch")
            except Exception as e:  # noqa: BLE001
                self.journal("warn", "preflight %s : %s" % (url, e))
                telemetrie.compte("cors.preflights_refuses")
                return cors.reponse_opaque(url)
            if not cors.preflight_accorde(mienne, methode, entetes,
                                          getattr(sonde, "entetes", {}) or {},
                                          sonde.code):
                self.journal("warn",
                             "CORS : le preflight de %s a ete refuse" % url)
                telemetrie.compte("cors.preflights_refuses")
                self.note(telemetrie.SOP_REFUS, "cors_preflight", url)
                return cors.reponse_opaque(url)
            telemetrie.compte("cors.preflights_accordes")

        return self._recupere_brut(methode, url, corps, entetes, mienne,
                                   avec_temoins)

    def _recupere_brut(self, methode, url, corps, entetes, mienne, avec_temoins):
        # `Origin` accompagne toute requete cross-origine. Sans lui, un serveur
        # ne peut pas repondre `Access-Control-Allow-Origin: <celle-la>` — il ne
        # sait pas a qui il parle —, et tout le mode « origine exacte » de CORS
        # devient inutilisable. C'est le navigateur qui le pose, jamais la page :
        # une page qui pourrait choisir son `Origin` contournerait la politique
        # entiere.
        entetes = dict(entetes or {})
        entetes.pop("origin", None)
        entetes.pop("Origin", None)
        if not mod_origine.Origine.de_url(url).meme_origine(mienne):
            entetes["Origin"] = mienne.serialise()
        try:
            # `brut=True` : une requete du script veut le corps tel quel, pas
            # la page d'habillage que le navigateur fabrique pour l'affichage.
            # Sans cela, un `fetch` de JSON recevait « Type non affichable ».
            reponse = reseau.charge(url, methode=methode, corps=corps,
                                    entetes=entetes or {}, brut=True,
                                    document=self.document.url,
                                    destination="fetch")
        except Exception as e:  # noqa: BLE001
            self.echec_ressource("fetch", url, 0, type(e).__name__)
            self.journal("warn", "requete %s : %s" % (url, e))
            return {"status": 0, "statusText": str(e), "text": "", "url": url,
                    "headers": {}}
        if not reponse.code or reponse.code >= 400:
            self.echec_ressource("fetch", reponse.url or url, reponse.code,
                                 reponse.erreur or "http")

        # La reponse est arrivee. Reste a savoir si le script a le droit de la
        # lire — c'est une question distincte de celle de l'envoi, et c'est ici
        # qu'elle se tranche.
        entetes_reponse = getattr(reponse, "entetes", {}) or {}
        lisible, raison = cors.reponse_lisible(mienne, reponse.url or url,
                                               entetes_reponse, avec_temoins)
        if not lisible:
            telemetrie.compte("cors.reponses_opaques")
            self.note(telemetrie.SOP_REFUS, "cors", raison)
            self.journal("warn", "CORS : %s" % raison)
            return cors.reponse_opaque(reponse.url or url)
        telemetrie.compte("cors.reponses_lisibles")
        return {
            "status": reponse.code,
            "statusText": reponse.erreur or "",
            "text": reponse.contenu,
            "url": reponse.url,
            # Meme lisible, une reponse cross-origine n'expose pas tous ses
            # en-tetes : un `Set-Cookie` ou un en-tete applicatif d'un autre
            # site n'ont pas a traverser.
            "headers": cors.filtre_entetes(entetes_reponse, mienne,
                                           reponse.url or url),
        }

    def _telecharge_script(self, source):
        absolue = urllib.parse.urljoin(self.document.url, source)
        try:
            # `brut` est indispensable : sans lui, le reseau enrobe tout ce qui
            # n'est pas du HTML dans un `<pre>` et en echappe les `&` et les
            # `<`. Chaque `<script src=…>` arrivait donc au moteur JavaScript
            # commencant par un chevron, et mourait sur le premier jeton.
            reponse = reseau.charge(absolue, brut=True,
                                    document=self.document.url,
                                    destination="script")
        except Exception as e:  # noqa: BLE001
            self.echec_ressource("script", absolue, 0, type(e).__name__)
            self.journal("warn", "script %s : %s" % (absolue, e))
            return ""
        if not reponse.code or reponse.code >= 400:
            # Le bundle principal d'une page passe par ici. Son absence vide la
            # page de tout comportement, et le rapport n'en disait rien.
            self.echec_ressource("script", absolue, reponse.code, "http")
            self.journal("warn", "script %s : code %s" % (absolue, reponse.code))
            return ""
        return reponse.contenu


# --- Utilitaires --------------------------------------------------------------

def _clone(nœud, profond):
    if isinstance(nœud, html.Texte):
        return html.Texte(nœud.contenu)
    copie = html.Element(nœud.balise, dict(nœud.attributs))
    if profond:
        for enfant in nœud.enfants:
            copie.ajoute(_clone(enfant, True))
    return copie


def _boite_de(boite, element):
    """Retrouve la boite posee pour cet element, si la mise en page en a une."""
    if boite is None:
        return None
    if boite.element is element:
        return boite
    for enfant in boite.enfants:
        trouvee = _boite_de(enfant, element)
        if trouvee is not None:
            return trouvee
    return None


def _chemin_prelude(nom="prelude.js"):
    import os
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), nom)


def _operation_toile(operation):
    """Traduit une operation de toile en element de liste d'affichage.

    Les couleurs arrivent en CSS et repartent en entier ARGB ; une operation
    mal formee est ecartee plutot que de faire tomber la peinture, parce que
    c'est du JavaScript de page qui l'a produite.
    """
    try:
        genre = operation[0]
        if genre == "rect":
            _, x, y, l, h, teinte = operation
            couleur = css.couleur(str(teinte))
            if couleur is None:
                return None
            return ("rect", float(x), float(y), float(l), float(h), couleur)
        if genre == "ligne":
            _, x1, y1, x2, y2, epaisseur, teinte = operation
            couleur = css.couleur(str(teinte))
            if couleur is None:
                return None
            return ("ligne", float(x1), float(y1), float(x2), float(y2),
                    float(epaisseur), couleur)
        if genre == "texte":
            _, x, y, texte, teinte, taille, gras, italique, fixe = operation
            couleur = css.couleur(str(teinte))
            if couleur is None:
                couleur = 0xFF000000
            return ("texte", float(x), float(y), str(texte), couleur,
                    float(taille), bool(gras), bool(italique), bool(fixe), False,
                    "")
        if genre == "image":
            _, x, y, l, h, identifiant = operation
            return ("image", float(x), float(y), float(l), float(h), int(identifiant))
        if genre == "polygone":
            _, points, teinte = operation
            couleur = css.couleur(str(teinte))
            if couleur is None or len(points) < 3:
                return None
            return ("polygone",
                    tuple((float(p[0]), float(p[1])) for p in points), couleur)
        if genre == "degrade_points":
            _, x, y, l, h, x0, y0, x1, y1, arrets = operation
            etapes = _arrets(arrets)
            if etapes is None:
                return None
            return ("degrade_points", float(x), float(y), float(l), float(h),
                    float(x0), float(y0), float(x1), float(y1), etapes)
        if genre == "degrade_cercle":
            _, x, y, l, h, cx, cy, rayon, arrets = operation
            etapes = _arrets(arrets)
            if etapes is None:
                return None
            return ("degrade_cercle", float(x), float(y), float(l), float(h),
                    float(cx), float(cy), float(rayon), etapes)
        if genre == "ombre":
            _, x, y, l, h, rayon, flou, teinte = operation
            couleur = css.couleur(str(teinte))
            if couleur is None:
                return None
            return ("ombre", float(x), float(y), float(l), float(h),
                    float(rayon), float(flou), couleur)
        if genre == "clip":
            _, x, y, l, h = operation
            return ("clip", float(x), float(y), float(l), float(h))
        if genre == "declip":
            return ("declip",)
    except (TypeError, ValueError, IndexError):
        return None
    return None


def _arrets(bruts):
    """Les arrets d'un degrade de toile, couleurs traduites. `None` si moins de deux."""
    etapes = []
    for position, teinte in bruts or []:
        couleur = css.couleur(str(teinte))
        if couleur is None:
            continue
        etapes.append((max(0.0, min(1.0, float(position))), couleur))
    return tuple(etapes) if len(etapes) >= 2 else None
