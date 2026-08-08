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

from . import css, html, reseau, securite, stockage, telemetrie

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
        self._lecteurs = {}        # identifiant de nœud -> Lecteur
        self._toiles = {}          # element <canvas> -> operations dessinees
        self._modules = {}         # adresse de module -> source (ou None)
        self._sources_mse = {}     # identifiant MediaSource -> Lecteur
        self._types_mse = {}       # identifiant MediaSource -> type MIME
        self._segments_mse = {}    # segments recus avant rattachement

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
            telemetrie.erreur_js(e, nom)
            self.journal("error", "%s : %s" % (nom, e))
        except Exception as e:  # noqa: BLE001 — une page ne doit pas tuer le navigateur
            telemetrie.erreur_js(e, nom)
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
        for lecteur in self._lecteurs.values():
            lecteur.ferme()
        self._lecteurs.clear()
        bojs.detruit(self._contexte)

    def _appelle(self, nom, arguments):
        try:
            return bojs.appelle(self._contexte, nom, arguments)
        except bojs.Erreur as e:
            telemetrie.erreur_js(e, nom)
            self.journal("error", str(e))
        except Exception as e:  # noqa: BLE001
            telemetrie.erreur_js(e, nom)
            self.journal("error", str(e))
        return None

    # --- Mesure ---------------------------------------------------------------

    def _op_manque(self, nom):
        """Un script est alle chercher une API que le moteur ne fournit pas.

        Appele depuis les accesseurs poses par le prelude. La valeur rendue
        reste `undefined` cote JavaScript : la mesure n'infléchit pas ce qui
        est mesure.
        """
        telemetrie.api_manquante(str(nom))
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
            self.journal("warn", "module %s : %s" % (adresse, e))
            self._modules[adresse] = None
            return None
        if reponse.code and reponse.code >= 400:
            self.journal("warn", "module %s : code %d" % (adresse, reponse.code))
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

    def _op_historique(self, pas):
        self.navigation = ("historique", int(pas))

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
            # `brut=True` : une requete du script veut le corps tel quel, pas
            # la page d'habillage que le navigateur fabrique pour l'affichage.
            # Sans cela, un `fetch` de JSON recevait « Type non affichable ».
            reponse = reseau.charge(url, methode=methode, corps=corps,
                                    entetes=entetes or {}, brut=True,
                                    document=self.document.url,
                                    destination="fetch")
        except Exception as e:  # noqa: BLE001
            telemetrie.ressource_echouee(url, 0, "fetch")
            self.journal("warn", "requete %s : %s" % (url, e))
            return {"status": 0, "statusText": str(e), "text": "", "url": url,
                    "headers": {}}
        if not reponse.code or reponse.code >= 400:
            telemetrie.ressource_echouee(url, reponse.code, "fetch")
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
            # `brut` est indispensable : sans lui, le reseau enrobe tout ce qui
            # n'est pas du HTML dans un `<pre>` et en echappe les `&` et les
            # `<`. Chaque `<script src=…>` arrivait donc au moteur JavaScript
            # commencant par un chevron, et mourait sur le premier jeton.
            reponse = reseau.charge(absolue, brut=True,
                                    document=self.document.url,
                                    destination="script")
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
