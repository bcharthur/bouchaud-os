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
import re
import time
import urllib.parse

import bo
import bojs

from . import html, reseau

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

_MAILLON = re.compile(r"""
    (?P<balise>[*a-zA-Z][a-zA-Z0-9_-]*)?
    (?P<reste>(?:
        \#[A-Za-z0-9_-]+
      | \.[A-Za-z0-9_-]+
      | \[[^\]]*\]
      | ::?[a-zA-Z-]+(?:\([^)]*\))?
    )*)
""", re.X)

_ATTRIBUT = re.compile(r"""\[\s*([A-Za-z_:][-A-Za-z0-9_:.]*)\s*
                           (?:([~^$*|]?=)\s*("[^"]*"|'[^']*'|[^\]]*?))?\s*\]""", re.X)


class _Simple:
    """Un maillon : balise, identifiant, classes, attributs."""

    __slots__ = ("balise", "identifiant", "classes", "attributs")

    def __init__(self, texte):
        self.balise = None
        self.identifiant = None
        self.classes = []
        self.attributs = []

        m = _MAILLON.match(texte)
        if m:
            balise = m.group("balise")
            if balise and balise != "*":
                self.balise = balise.lower()
            reste = m.group("reste") or ""
        else:
            reste = texte

        for m in _ATTRIBUT.finditer(reste):
            nom, operateur, valeur = m.group(1).lower(), m.group(2), m.group(3)
            if valeur and valeur[:1] in "\"'":
                valeur = valeur[1:-1]
            self.attributs.append((nom, operateur, valeur))
        reste = _ATTRIBUT.sub("", reste)
        # Les pseudo-classes sont ignorees, comme dans la cascade : `:hover`
        # demanderait un etat d'interaction que le moteur ne tient pas.
        reste = re.sub(r"::?[a-zA-Z-]+(\([^)]*\))?", "", reste)
        for morceau in re.findall(r"[.#][A-Za-z0-9_-]+", reste):
            if morceau[0] == ".":
                self.classes.append(morceau[1:])
            else:
                self.identifiant = morceau[1:]

    def correspond(self, element):
        if self.balise and element.balise != self.balise:
            return False
        if self.identifiant and element.attributs.get("id") != self.identifiant:
            return False
        if self.classes:
            presentes = set(element.attributs.get("class", "").split())
            if not all(c in presentes for c in self.classes):
                return False
        for nom, operateur, valeur in self.attributs:
            if nom not in element.attributs:
                return False
            if operateur is None:
                continue
            presente = element.attributs[nom]
            if operateur == "=" and presente != valeur:
                return False
            if operateur == "^=" and not presente.startswith(valeur):
                return False
            if operateur == "$=" and not presente.endswith(valeur):
                return False
            if operateur == "*=" and valeur not in presente:
                return False
            if operateur == "~=" and valeur not in presente.split():
                return False
            if operateur == "|=" and presente != valeur \
                    and not presente.startswith(valeur + "-"):
                return False
        return True


class _Selecteur:
    """Une suite de maillons, avec la distinction descendant / enfant direct.

    Chaque etape porte le lien qui la rattache a la **precedente** : `div > p`
    donne `[(div, descendant), (p, enfant)]`. C'est ce qui permet de verifier
    de droite a gauche — le sens naturel, puisqu'on part de l'element candidat.
    """

    __slots__ = ("etapes",)

    def __init__(self, texte):
        # `+` et `~` (freres) sont ramenes a la descendance : approximatif, mais
        # plus proche du resultat attendu que de rejeter la regle.
        texte = re.sub(r"\s*[+~]\s*", " ", texte.strip())
        self.etapes = []
        enfant_direct = False
        for morceau in re.split(r"\s*(>)\s*|\s+", texte):
            if not morceau:
                continue
            if morceau == ">":
                enfant_direct = True
                continue
            self.etapes.append((_Simple(morceau), enfant_direct))
            enfant_direct = False

    def correspond(self, element, parent_de):
        """`parent_de` rend le parent d'un element (ou None)."""
        if not self.etapes:
            return False
        if not self.etapes[-1][0].correspond(element):
            return False

        courant = element
        for index in range(len(self.etapes) - 2, -1, -1):
            maillon = self.etapes[index][0]
            # Le lien a verifier est celui porte par l'etape de droite.
            direct = self.etapes[index + 1][1]
            courant = parent_de(courant)
            if direct:
                if courant is None or not maillon.correspond(courant):
                    return False
                continue
            while courant is not None and not maillon.correspond(courant):
                courant = parent_de(courant)
            if courant is None:
                return False
        return True


def _groupes(selecteur):
    return [_Selecteur(m) for m in selecteur.split(",") if m.strip()]


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

class Contexte:
    """Le JavaScript d'un document : son interprete, ses nœuds, ses minuteries."""

    def __init__(self, document, journal=None, budget_ms=5000):
        self.document = document
        self.journal = journal or (lambda niveau, texte: None)
        self.sale = False
        self.navigation = None

        self._contexte = bojs.cree(budget_ms)
        bojs.pont(self._contexte, self.appel)

        self._identifiants = {}    # id(nœud) -> entier
        self._noeuds = {}          # entier -> nœud
        self._prochain = 1

        self._minuteries = {}      # identifiant -> (echeance, delai, repete)
        self._reponses = []        # (identifiant, reponse) a livrer au battement

        with open(_chemin_prelude(), "r", encoding="utf-8") as f:
            bojs.evalue(self._contexte, f.read(), "prelude.js")

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
            self.execute(code, source or "<page>")

    def execute(self, code, nom="<page>"):
        """Execute du code, en signalant les erreurs plutot qu'en tombant."""
        try:
            return bojs.evalue(self._contexte, code, nom)
        except bojs.Erreur as e:
            self.journal("error", "%s : %s" % (nom, e))
        except Exception as e:  # noqa: BLE001 — une page ne doit pas tuer le navigateur
            self.journal("error", "%s : %s" % (nom, e))
        return None

    def signale_pret(self):
        """Declenche `DOMContentLoaded` puis `load`."""
        self._appelle("__bo_pret", [])

    def tic(self):
        """A appeler regulierement : minuteries echues et reponses en attente."""
        for identifiant, reponse in self._reponses[:]:
            self._reponses.remove((identifiant, reponse))
            self._appelle("__bo_reponse", [identifiant, reponse])

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

        try:
            bojs.pompe(self._contexte)
        except Exception:
            pass

    def evenement(self, nœud, type_, details=None):
        """Distribue un evenement reel (clic, touche) dans la page."""
        identifiant = None if nœud is None else self._identifiant(nœud)
        return self._appelle("__bo_evenement", [identifiant, type_, details or {}])

    def ferme(self):
        bojs.detruit(self._contexte)

    def _appelle(self, nom, arguments):
        try:
            return bojs.appelle(self._contexte, nom, arguments)
        except bojs.Erreur as e:
            self.journal("error", str(e))
        except Exception as e:  # noqa: BLE001
            self.journal("error", str(e))
        return None

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

    def appel(self, operation, *arguments):
        """Point d'entree de `__bo_appel`, cote Python."""
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
        self.sale = True

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
        parent_de = lambda e: e.parent  # noqa: E731 — lisible tel quel
        trouves = []
        for nœud in racine.parcours():
            if not isinstance(nœud, html.Element):
                continue
            if any(g.correspond(nœud, parent_de) for g in groupes):
                if not tous:
                    return self._identifiant(nœud)
                trouves.append(self._identifiant(nœud))
        return trouves if tous else None

    def _op_correspond(self, identifiant, selecteur):
        element = self._element(identifiant)
        if not element:
            return False
        parent_de = lambda e: e.parent  # noqa: E731
        return any(g.correspond(element, parent_de) for g in _groupes(selecteur))

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

    def _op_historique(self, pas):
        self.navigation = ("historique", int(pas))

    def _op_tailleVue(self):
        largeur, hauteur = bo.taille()
        return {"width": largeur, "height": hauteur}

    def _op_base64(self, texte, encode):
        if encode:
            return base64.b64encode(texte.encode("latin-1", "replace")).decode("ascii")
        return base64.b64decode(texte.encode("ascii")).decode("latin-1", "replace")

    # --- Operations : minuteries et reseau ------------------------------------

    def _op_minuterie(self, identifiant, delai, repete):
        maintenant = time.monotonic() * 1000.0
        self._minuteries[int(identifiant)] = (maintenant + delai, delai, bool(repete))

    def _op_annuleMinuterie(self, identifiant):
        self._minuteries.pop(int(identifiant), None)

    def _op_requete(self, identifiant, methode, url, corps, entetes, synchrone):
        absolue = urllib.parse.urljoin(self.document.url, url)
        reponse = self._recupere(methode, absolue, corps, entetes)
        if synchrone:
            self._appelle("__bo_reponse", [int(identifiant), reponse])
        else:
            # Livree au battement suivant : une reponse rendue pendant l'appel
            # ferait tourner le code de retour avant que `send()` ait rendu la
            # main, ce qu'aucune page n'attend.
            self._reponses.append((int(identifiant), reponse))

    def _recupere(self, methode, url, corps, entetes):
        try:
            reponse = reseau.charge(url, methode=methode, corps=corps,
                                    entetes=entetes or {})
        except Exception as e:  # noqa: BLE001
            self.journal("warn", "requete %s : %s" % (url, e))
            return {"status": 0, "statusText": str(e), "text": "", "url": url,
                    "headers": {}}
        return {
            "status": reponse.code,
            "statusText": reponse.erreur or "",
            "text": reponse.contenu,
            "url": reponse.url,
            "headers": getattr(reponse, "entetes", {}) or {},
        }

    def _telecharge_script(self, source):
        absolue = urllib.parse.urljoin(self.document.url, source)
        try:
            reponse = reseau.charge(absolue)
        except Exception as e:  # noqa: BLE001
            self.journal("warn", "script %s : %s" % (absolue, e))
            return ""
        if reponse.code and reponse.code >= 400:
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


def _chemin_prelude():
    import os
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "prelude.js")
