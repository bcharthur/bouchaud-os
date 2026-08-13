"""Ce que le chrome voit d'une page, quel que soit l'endroit ou elle vit.

## Le probleme

Le navigateur avait deux moteurs sous la main et n'en utilisait qu'un. Ouvrir
une page depuis l'interface faisait :

    Navigateur.ouvre() -> Document(reseau.charge(url), ...)

c'est-a-dire dans le processus de la fenetre, avec le DOM a portee d'attribut.
Le processus separe de `superviseur.py` existait, marchait, et n'etait exerce
que par les epreuves. L'architecture affichee et l'architecture executee
n'etaient pas la meme.

Recoller les deux ne pouvait pas se faire en changeant une ligne, parce que le
chrome ne se contentait pas d'afficher un document : il le **lisait**. Le survol
demandait `document.lien_a(...)`, le clavier demandait `document.foyer_actuel()`,
l'ascenseur demandait `document.hauteur`, le clic appelait
`document.clic_complet(...)` et regardait ce qu'il rendait. Chacune de ces
lectures est impossible a travers une frontiere de processus.

## La forme de la solution

Une seule interface, deux implantations :

    VueLocale     le document dans ce processus       (regression, debug)
    VueRenderer   le document dans un autre processus (defaut)

Le chrome ne parle plus qu'a cette interface. Il ne lit plus de DOM : il pousse
des evenements et lit des **etats** que la vue tient a jour — titre, adresse,
curseur, lien survole, foyer, hauteur du document. Pour `VueLocale` ces etats
sont remplis en interrogeant le document ; pour `VueRenderer` ils arrivent par
messages. Le chrome, lui, ne voit aucune difference, ce qui est la seule facon
de pouvoir comparer les deux sur le meme scenario.

## Le vocabulaire des evenements

`bat()` rend une liste de `(nom, charge)`. Les noms sont ceux du chrome, pas
ceux du protocole — un chrome qui connaitrait `URL_CHANGED` connaitrait le
protocole, et n'aurait pas gagne grand-chose a la separation :

    TRAME        il y a de quoi redessiner
    TITRE        le titre a change
    URL          l'adresse a change
    CURSEUR      la forme du pointeur, et le lien sous lui
    NAVIGUE      la page demande a aller ailleurs — **le chrome decide**
    HISTORIQUE   la page demande `history.go(n)` — le chrome decide
    CRASH        le renderer est mort ; la fenetre, non
    JOURNAL      un message de console

## Ce qui reste au chrome, et pourquoi

L'historique et la barre d'adresse. Une navigation demandee par la page remonte
en `NAVIGUE` et **n'est pas appliquee** par la vue : c'est le chrome qui appelle
`ouvre()`, donc lui qui empile. Laisser le renderer naviguer seul aurait donne
une page a l'ecran sans entree dans l'historique — un bouton « reculer » qui
ment. La regle « le renderer demande, le navigateur decide » n'est pas seulement
une regle de securite : c'est aussi la seule qui garde une seule verite sur ou
l'on se trouve.
"""

import os
import time

import bo

from . import protocole, reseau, superviseur

# Ce que le chrome laisse a une page pour se charger avant de rendre la main.
# Assez pour une page ordinaire sous emulation, assez court pour qu'un serveur
# muet ne gele pas la fenetre pour toujours.
DELAI_CHARGEMENT_S = 30.0


def en_processus():
    """Vrai si l'utilisateur a demande l'ancien chemin en-processus.

    Une variable d'environnement et pas une option : c'est un outil de
    comparaison, pas un choix offert. Le jour ou il faut savoir si un defaut
    vient du moteur ou de la separation, on relance avec, et on compare.
    """
    return os.environ.get("BO_BROWSER_INPROCESS", "") not in ("", "0")


def ouvre_vue(largeur, hauteur, journal=None, force=None):
    """La vue que le navigateur doit utiliser. **Separee par defaut.**

    `force` vaut `"local"` ou `"renderer"` pour les epreuves, qui veulent
    eprouver les deux sans dependre de l'environnement.
    """
    choix = force or ("local" if en_processus() else "renderer")
    if choix == "local":
        return VueLocale(largeur, hauteur, journal=journal)
    return VueRenderer(largeur, hauteur, journal=journal)


class _Vue:
    """L'etat commun aux deux implantations, et rien de plus."""

    genre = "?"

    def __init__(self, largeur, hauteur, journal=None):
        self.largeur = max(1, int(largeur))
        self.hauteur = max(1, int(hauteur))
        self.journal = journal or (lambda niveau, texte: None)
        self.url = ""
        self.titre = ""
        self.erreur = None
        self.crashee = False
        self.curseur = "fleche"
        self.lien_survole = None
        self.foyer = False
        self.defilement = 0.0
        self.hauteur_document = 0.0
        self.derniers_evenements = []

    # Ce que le chrome appelle et que les deux doivent tenir.
    def ouvre(self, url):
        raise NotImplementedError

    def bat(self):
        return []

    def ferme(self):
        pass

    def defilement_maximum(self):
        return max(0.0, self.hauteur_document - self.hauteur + 40)


class VueLocale(_Vue):
    """Le document dans ce processus. L'ancien chemin, conserve tel quel.

    Il ne sert plus par defaut. Il reste parce qu'un moteur qui ne tourne que
    d'un cote d'une frontiere de processus est un moteur dont on ne sait pas
    isoler les defauts : quand une page se comporte mal, la question « est-ce
    le moteur ou la separation ? » se tranche en relancant avec
    `BO_BROWSER_INPROCESS=1`.
    """

    genre = "local"

    def __init__(self, largeur, hauteur, journal=None):
        super().__init__(largeur, hauteur, journal=journal)
        self.document = None

    # --- Cycle de vie ---------------------------------------------------------

    def ouvre(self, url):
        # Import tardif : `moteur/__init__` importe la moitie du moteur, et
        # l'importer au chargement de ce module ferait un cycle avec le paquet
        # qui le contient.
        from . import Document

        reponse = reseau.charge(url)
        document = None
        try:
            document = Document(reponse, self.largeur, journal=self.journal,
                                hauteur_fenetre=self.hauteur)
        except Exception as e:  # noqa: BLE001 — une page cassee ne ferme pas la fenetre
            self.erreur = str(e)
            return False
        if self.document is not None and self.document is not document:
            self.document.ferme()
        self.document = document
        self.defilement = 0.0
        self.url = document.url
        self.titre = document.titre or document.url
        self.erreur = document.erreur
        self.hauteur_document = float(document.hauteur)
        return True

    def ferme(self):
        if self.document is not None:
            self.document.ferme()
            self.document = None

    # --- Geometrie ------------------------------------------------------------

    def redimensionne(self, largeur, hauteur):
        self.largeur, self.hauteur = max(1, int(largeur)), max(1, int(hauteur))
        if self.document is not None:
            self.document.remet_en_page(self.largeur, self.hauteur)
            self.hauteur_document = float(self.document.hauteur)

    def defile(self, pixels):
        if self.document is None:
            return
        self.defilement = min(self.defilement_maximum(),
                              max(0.0, self.defilement + pixels))
        # Le document connait sa position : c'est ce que lisent
        # `IntersectionObserver` et `window.scrollY`.
        self.document.defilement = self.defilement

    def va_en_haut(self):
        self.defilement = 0.0
        if self.document is not None:
            self.document.defilement = 0.0

    # --- Entrees --------------------------------------------------------------

    def souris(self, x, y):
        if self.document is None:
            return []
        page_y = y + self.defilement
        change = self.document.survole(x, page_y)
        lien = self.document.lien_a(x, page_y)
        forme = "pointeur" if lien else "fleche"
        evenements = []
        if forme != self.curseur or lien != self.lien_survole:
            self.curseur, self.lien_survole = forme, lien
            evenements.append(("CURSEUR", {"forme": forme, "lien": lien}))
        if change:
            evenements.append(("TRAME", {}))
        return evenements

    def clic(self, x, y):
        if self.document is None:
            return []
        page_y = y + self.defilement
        suite, lien = self.document.clic_complet(
            x, page_y, {"clientX": x, "clientY": y, "pageX": x, "pageY": page_y})
        self.foyer = self.document.foyer_actuel() is not None
        evenements = [("TRAME", {})]
        if suite != "annule" and lien:
            evenements.append(("NAVIGUE", {"url": lien}))
        return evenements

    def touche(self, nom, texte="", maj=False, ctrl=False):
        if self.document is None:
            return []
        self.document.frappe(nom, texte, maj=maj, ctrl=ctrl)
        self.foyer = self.document.foyer_actuel() is not None
        return [("TRAME", {})]

    # --- Battement ------------------------------------------------------------

    def bat(self):
        document = self.document
        if document is None:
            return []
        evenements = []
        if document.rafraichis():
            evenements.append(("TRAME", {}))
        self.hauteur_document = float(document.hauteur)
        if document.titre and document.titre != self.titre:
            self.titre = document.titre
            evenements.append(("TITRE", {"titre": self.titre}))
        if document.url != self.url:
            self.url = document.url
            evenements.append(("URL", {"url": self.url}))
        demande = document.navigation_demandee
        if isinstance(demande, tuple):
            evenements.append(("HISTORIQUE", {"pas": demande[1]}))
        elif demande:
            evenements.append(("NAVIGUE", {"url": demande}))
        return evenements

    # --- Peinture -------------------------------------------------------------

    def peint(self, liste, y0):
        """Ajoute la page a la liste d'affichage du chrome."""
        if self.document is None:
            return
        liste.extend(self.document.liste_affichage(
            self.defilement - y0, self.largeur, self.hauteur + y0))


class VueRenderer(_Vue):
    """Le document dans un autre processus. **Le chemin par defaut.**

    Tout ce que le chrome sait de la page arrive par le canal de controle ou par
    la surface partagee. Aucune methode ici ne touche un DOM : c'est verifiable
    a la lecture, et c'est le point.
    """

    genre = "renderer"

    def __init__(self, largeur, hauteur, journal=None, courtage=True):
        super().__init__(largeur, hauteur, journal=journal)
        self.courtage = courtage
        self.renderer = None
        self.image = None            # (identifiant, largeur, hauteur) cote Qt
        self.generation = 0
        self.trames = 0
        self._demarre()

    def _demarre(self):
        self.renderer = superviseur.Renderer(
            self.largeur, self.hauteur, journal=self.journal,
            courtage=self.courtage)
        # Le chrome tient l'historique : c'est lui qui applique une navigation
        # autorisee, pas le superviseur. Voir l'en-tete du module.
        self.renderer.auto_navigation = False
        self.crashee = False
        self._pompe(5.0, attendu="READY")

    # --- Traduction -----------------------------------------------------------

    def _traduis(self, brut):
        """Des messages du protocole aux evenements du chrome."""
        evenements = []
        for nom, charge in brut:
            charge = charge or {}
            if nom == "TITLE_CHANGED":
                self.titre = charge.get("titre") or self.titre
                evenements.append(("TITRE", {"titre": self.titre}))
            elif nom == "URL_CHANGED":
                self.url = charge.get("url") or self.url
                evenements.append(("URL", {"url": self.url}))
            elif nom == "CURSOR_CHANGED":
                self.curseur = charge.get("forme", "fleche")
                self.lien_survole = charge.get("lien")
                evenements.append(("CURSEUR", {"forme": self.curseur,
                                               "lien": self.lien_survole}))
            elif nom == "FOCUS_CHANGED":
                self.foyer = bool(charge.get("foyer"))
            elif nom == "FRAME_READY":
                self.hauteur_document = float(charge.get("hauteur_document") or 0.0)
                self.generation = int(charge.get("generation") or 0)
                self._prend_trame()
                evenements.append(("TRAME", charge))
            elif nom == "NAVIGATION_AUTORISEE":
                evenements.append(("NAVIGUE", {"url": charge.get("url")}))
            elif nom == "NAVIGATION_HISTORIQUE":
                evenements.append(("HISTORIQUE", {"pas": charge.get("pas")}))
            elif nom == "NAVIGATION_REFUSEE":
                evenements.append(("JOURNAL", {
                    "niveau": "warn",
                    "texte": "navigation refusee : %s" % charge.get("url")}))
            elif nom == "CONSOLE_MESSAGE":
                evenements.append(("JOURNAL", {
                    "niveau": charge.get("niveau", "log"),
                    "texte": charge.get("texte", "")}))
            elif nom == "ERROR":
                evenements.append(("JOURNAL", {
                    "niveau": "error",
                    "texte": "%s : %s" % (charge.get("raison"),
                                          charge.get("detail"))}))
            elif nom == "CRASH":
                self._encaisse_crash(charge)
                evenements.append(("CRASH", charge))
            elif nom == "PROTOCOLE":
                self._encaisse_crash({"raison": "protocole : %s"
                                      % charge.get("detail")})
                evenements.append(("CRASH", charge))
        return evenements

    def _pompe(self, secondes=0.0, attendu=None):
        """Bat, ecoute, et traduit. Rend les evenements du chrome."""
        if self.renderer is None:
            return []
        if attendu is None:
            try:
                self.renderer.bat()
            except protocole.Fin:
                pass
            return self._traduis(self.renderer.recolte(secondes))
        _vu, brut = self.renderer.attends(attendu, secondes)
        return self._traduis(brut)

    # --- Cycle de vie ---------------------------------------------------------

    def ouvre(self, url):
        """Demande la page, et attend d'en avoir quelque chose a montrer.

        L'attente est bornee et le chrome reste peint pendant : un renderer qui
        ne repond pas laisse une fenetre vivante avec une barre d'etat qui le
        dit, ce qui est tout l'interet d'avoir mis la page ailleurs.
        """
        if self.crashee or self.renderer is None or not self.renderer.vivant():
            self._redemarre()
        self.erreur = None
        try:
            self.renderer.navigue(url)
        except (protocole.Fin, OSError) as e:
            self.erreur = str(e)
            return False
        self.defilement = 0.0
        # L'URL d'abord — elle vient avant les pixels, et c'est elle qui remplit
        # la barre d'adresse pendant que la page se peint.
        vus = self._attends_parmi(("URL", "TRAME", "CRASH"), DELAI_CHARGEMENT_S)
        vus += self._attends_parmi(("TRAME", "CRASH"), DELAI_CHARGEMENT_S)
        self.derniers_evenements = vus
        return not self.crashee

    def _attends_parmi(self, noms, secondes):
        vus = []
        limite = time.monotonic() + secondes
        while time.monotonic() < limite:
            lot = self._pompe(0.02)
            vus.extend(lot)
            if any(nom in noms for nom, _ in lot):
                return vus
            if self.crashee:
                return vus
        return vus

    def _redemarre(self):
        if self.renderer is not None:
            try:
                self.renderer.ferme()
            except Exception:  # noqa: BLE001
                pass
            self.renderer = None
        self._demarre()

    def ferme(self):
        if self.renderer is not None:
            try:
                self.renderer.ferme()
            finally:
                self.renderer = None

    def _encaisse_crash(self, charge):
        """La page est morte. La fenetre continue, avec sa derniere image.

        L'image survit parce qu'elle a deja ete copiee chez Qt : la surface
        partagee peut etre liberee sans que l'ecran se vide. C'est ce qui fait
        la difference entre « le navigateur a plante » et « cette page a plante ».
        """
        if self.crashee:
            return
        self.crashee = True
        self.erreur = charge.get("raison") or "renderer mort"
        self.foyer = False
        self.curseur = "fleche"
        self.lien_survole = None
        if self.renderer is not None:
            # Recolte et liberation : le processus est deja recolte par
            # `superviseur.vivant()`, il reste la surface et la prise.
            try:
                self.renderer.ferme()
            except Exception:  # noqa: BLE001
                pass

    # --- Geometrie ------------------------------------------------------------

    def redimensionne(self, largeur, hauteur):
        largeur, hauteur = max(1, int(largeur)), max(1, int(hauteur))
        if (largeur, hauteur) == (self.largeur, self.hauteur):
            return
        self.largeur, self.hauteur = largeur, hauteur
        if self.renderer is None or self.crashee:
            return
        try:
            self.renderer.redimensionne(largeur, hauteur)
        except (protocole.Fin, OSError) as e:
            self.journal("warn", "redimensionnement perdu : %s" % e)

    def defile(self, pixels):
        self.defilement = min(self.defilement_maximum(),
                              max(0.0, self.defilement + pixels))
        self._pousse_defilement()

    def va_en_haut(self):
        self.defilement = 0.0
        self._pousse_defilement()

    def _pousse_defilement(self):
        if self.renderer is None or self.crashee:
            return
        try:
            self.renderer.defile(self.defilement)
        except (protocole.Fin, OSError):
            pass

    # --- Entrees --------------------------------------------------------------

    def souris(self, x, y):
        if self.renderer is None or self.crashee:
            return []
        try:
            self.renderer.souris(x, y)
        except (protocole.Fin, OSError):
            return []
        return self._pompe(0.0)

    def clic(self, x, y):
        if self.renderer is None or self.crashee:
            return []
        try:
            self.renderer.clic(x, y)
        except (protocole.Fin, OSError):
            return []
        # Un clic peut declencher un script, donc une requete courtee, donc un
        # aller-retour. On laisse un peu de temps a la reponse plutot que de
        # rendre la main sur une page a moitie a jour.
        return self._pompe(0.05)

    def touche(self, nom, texte="", maj=False, ctrl=False):
        if self.renderer is None or self.crashee:
            return []
        try:
            self.renderer.touche(nom, texte, maj=maj, ctrl=ctrl)
        except (protocole.Fin, OSError):
            return []
        return self._pompe(0.05)

    # --- Battement ------------------------------------------------------------

    def bat(self):
        if self.renderer is None or self.crashee:
            return []
        return self._pompe(0.0)

    # --- Peinture -------------------------------------------------------------

    def _prend_trame(self):
        """Copie la trame publiee chez Qt, en reutilisant l'emplacement.

        Reutiliser l'identifiant est ce qui rend le rythme tenable : une trame
        par seizieme de seconde ferait grossir le cache d'images de neuf cents
        entrees par minute.
        """
        if self.renderer is None:
            return
        octets = self.renderer.trame()
        if not octets:
            return
        largeur = self.renderer.surface.largeur
        hauteur = self.renderer.surface.hauteur
        identifiant = self.image[0] if self.image else -1
        try:
            self.image = bo.image_brute(octets, largeur, hauteur, identifiant)
        except Exception as e:  # noqa: BLE001
            self.journal("warn", "trame illisible : %s" % e)
            return
        self.trames += 1

    def peint(self, liste, y0):
        if self.image is None:
            return
        identifiant, largeur, hauteur = self.image
        liste.append(("image", 0, y0, largeur, hauteur, identifiant))
