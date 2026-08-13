"""Le processus de rendu : un moteur web au bout d'une prise.

## Le partage des roles

Le navigateur garde ce qui ne doit jamais mourir : la fenetre, le chrome, les
entrees materielles, le cycle de vie des processus, et **la politique** —
temoins, stockage, decision de naviguer. Le renderer prend ce qui peut mourir :
l'analyse HTML, la cascade CSS, le DOM, la mise en page, JavaScript.

La ligne est celle-ci : *un renderer compromis ne doit gagner que ce que sa page
pouvait deja faire*. Il n'ouvre pas de fenetre, ne lit pas de temoin, ne decide
pas d'une navigation — il la **demande** (`REQUEST_NAVIGATION`), et le
navigateur applique.

## Ce qui traverse, et par ou

Le canal de controle porte des messages courts : navigue, redimensionne, voici
une touche, bats une fois. La surface partagee porte les pixels. Rien d'autre
ne circule — en particulier, la liste d'affichage ne traverse pas : elle est
jouee **ici**, dans ce processus, et seul son resultat en pixels est publie.

C'est un choix, et il merite d'etre dit. Chromium fait l'inverse : il envoie
une liste d'affichage que le processus GPU rasterise. C'est mieux quand il y a
un GPU et un compositeur ; ici il n'y en a pas, et rasteriser sur place evite
d'inventer un encodage binaire pour des tuples Python — le seul morceau du
protocole que `BROWSER_RENDERER_PROTOCOL.md` designait comme du vrai travail.

## Le rythme

Le renderer ne bat pas tout seul. `TICK` vient du navigateur, et c'est ce qui
permet de geler un onglet en arriere-plan sans le tuer — et ce qui rend une
epreuve reproductible, puisque rien n'avance entre deux battements demandes.
"""

import os
import resource
import socket
import time

import bo

from . import protocole, surface as mod_surface

# Le renderer se coupe si le navigateur ne lui parle plus. Sans cela, un
# navigateur tue laisserait derriere lui un processus qui peint pour personne.
SILENCE_MAX_S = 120.0


class Renderer:
    """L'etat d'un renderer : un document, une surface, une prise."""

    def __init__(self, prise):
        self.prise = prise
        self.canal = protocole.Canal(prise)
        self.surface = None
        self.document = None
        self.contexte = 0
        self.largeur = 800
        self.hauteur = 600
        self.defilement = 0.0
        self.generation = 0
        self.tampon = 0
        self.titre = None
        self.url = None
        self.a_repeindre = False

    # --- Sortie ---------------------------------------------------------------

    def dis(self, genre, charge=None):
        try:
            self.canal.envoie(genre, charge)
        except OSError:
            raise protocole.Fin()

    def journal(self, niveau, texte):
        try:
            self.dis(protocole.CONSOLE_MESSAGE,
                     {"contexte": self.contexte, "niveau": str(niveau),
                      "texte": str(texte)})
        except protocole.Fin:
            pass

    # --- Boucle ---------------------------------------------------------------

    def sers(self):
        """Lit des messages jusqu'a `CLOSE`, une fin de prise, ou le silence."""
        self.prise.settimeout(SILENCE_MAX_S)
        # Le renderer annonce le bac a sable dans lequel il s'est reveille. Ce
        # n'est pas de la politesse : c'est la seule facon pour le navigateur de
        # verifier que la limite qu'il a posee est bien celle qui s'applique de
        # l'autre cote. Une limite posee et non appliquee — parce qu'un
        # `setrlimit` a echoue en silence, parce que le systeme l'ignore — se
        # lit exactement comme une limite appliquee, jusqu'au jour ou un
        # renderer emporte la machine.
        try:
            douce, _dure = resource.getrlimit(resource.RLIMIT_AS)
        except (ValueError, OSError):
            douce = -1
        self.dis(protocole.READY, {"version": protocole.VERSION,
                                   "pid": os.getpid(),
                                   "limite_as": douce})
        while True:
            try:
                genre, charge = self.canal.lis(protocole.VERS_RENDERER)
            except protocole.Fin:
                return 0
            except socket.timeout:
                return 0
            except protocole.Erreur as e:
                # Une trame illisible est fatale a la connexion. Continuer
                # reviendrait a lire la suite du flux a partir d'une position
                # dont on ne sait plus rien.
                self.dis(protocole.ERROR, {"raison": "protocole", "detail": str(e)})
                return 2
            if genre == protocole.CLOSE:
                return 0
            try:
                self.traite(genre, charge)
            except protocole.Fin:
                return 0
            except Exception as e:  # noqa: BLE001 — une page ne tue pas le renderer
                self.dis(protocole.ERROR,
                         {"raison": protocole.NOMS.get(genre, str(genre)),
                          "detail": "%s: %s" % (type(e).__name__, e)})

    def traite(self, genre, charge):
        charge = charge or {}
        if genre == protocole.SURFACE:
            self.recoit_surface(charge)
        elif genre == protocole.CREATE_DOCUMENT:
            self.contexte = int(charge.get("contexte", 0))
            self.largeur = int(charge.get("largeur", self.largeur))
            self.hauteur = int(charge.get("hauteur", self.hauteur))
        elif genre == protocole.NAVIGATE:
            self.navigue(str(charge.get("url", "")))
        elif genre == protocole.RESIZE:
            self.redimensionne(int(charge.get("largeur", self.largeur)),
                               int(charge.get("hauteur", self.hauteur)))
        elif genre == protocole.INPUT_EVENT:
            self.entree(charge)
        elif genre == protocole.TICK:
            self.bat()

    # --- Surface --------------------------------------------------------------

    def recoit_surface(self, charge):
        """Recupere le descripteur envoye par `SCM_RIGHTS`.

        Le message de controle annonce les dimensions ; le descripteur voyage a
        cote, dans les donnees auxiliaires. Les deux arrivent ensemble parce
        que `sendmsg` les a mis dans le meme envoi — c'est la seule facon de ne
        pas avoir a les reapparier.
        """
        descripteur = self.canal.prends_descripteur()
        if descripteur is None:
            raise protocole.Erreur("descripteur de surface non recu")
        if self.surface is not None:
            self.surface.ferme()
        self.surface = mod_surface.Surface(descripteur,
                                           int(charge["largeur"]),
                                           int(charge["hauteur"]))
        self.largeur = self.surface.largeur
        self.hauteur = self.surface.hauteur
        self.a_repeindre = True

    # --- Document -------------------------------------------------------------

    def navigue(self, url):
        # Import tardif : `moteur` charge le monde entier — cascade, mise en
        # page, JavaScript. Le renderer n'en a besoin qu'a la premiere
        # navigation, et un enfant qu'on tue avant celle-ci n'aura rien paye.
        import moteur

        if self.document is not None:
            self.document.ferme_document()
            self.document = None
        self.document = moteur.charge(url, self.largeur, journal=self.journal)
        self.document.remet_en_page(self.largeur, self.hauteur)
        self.url = self.document.url
        self.dis(protocole.URL_CHANGED,
                 {"contexte": self.contexte, "url": self.url})
        self.annonce_titre()
        self.a_repeindre = True

    def redimensionne(self, largeur, hauteur):
        self.largeur, self.hauteur = largeur, hauteur
        if self.document is not None:
            self.document.remet_en_page(largeur, hauteur)
        self.a_repeindre = True

    def annonce_titre(self):
        titre = getattr(self.document, "titre", None)
        if titre and titre != self.titre:
            self.titre = titre
            self.dis(protocole.TITLE_CHANGED,
                     {"contexte": self.contexte, "titre": titre})

    def entree(self, charge):
        """Souris, clavier, defilement.

        Le renderer ne lit **jamais** le materiel : il recoit des evenements
        deja normalises par le navigateur. C'est ce qui lui permet de n'avoir
        aucun droit sur les peripheriques, et c'est aussi ce qui rend une
        epreuve capable de rejouer une session sans clavier.
        """
        if self.document is None:
            return
        genre = str(charge.get("genre", ""))
        if genre == "souris":
            if self.document.survole(float(charge.get("x", 0)),
                                     float(charge.get("y", 0))):
                self.a_repeindre = True
            forme = "pointeur" if self.document.lien_a(
                float(charge.get("x", 0)),
                float(charge.get("y", 0)) + self.defilement) else "fleche"
            self.dis(protocole.CURSOR_CHANGED,
                     {"contexte": self.contexte, "forme": forme})
        elif genre == "clic":
            x = float(charge.get("x", 0))
            y = float(charge.get("y", 0)) + self.defilement
            # Exactement la sequence du chrome, parce que c'est la meme
            # methode : foyer, evenement, action par defaut, lien.
            _suite, url = self.document.clic_complet(
                x, y, {"clientX": charge.get("x", 0),
                       "clientY": charge.get("y", 0),
                       "pageX": x, "pageY": y})
            self.a_repeindre = True
            if url:
                # Le renderer demande, il ne navigue pas. C'est le navigateur
                # qui applique la politique — schema autorise, origine,
                # historique —, et un renderer compromis ne doit pas pouvoir
                # s'en passer.
                self.dis(protocole.REQUEST_NAVIGATION,
                         {"contexte": self.contexte, "url": url,
                          "provenance": self.url})
        elif genre == "touche":
            self.document.frappe(str(charge.get("touche", "")),
                                 str(charge.get("texte", "")),
                                 bool(charge.get("maj")),
                                 bool(charge.get("ctrl")))
            self.a_repeindre = True
        elif genre == "defilement":
            self.defilement = max(0.0, float(charge.get("position", 0)))
            self.a_repeindre = True

    def bat(self):
        if self.document is None:
            return
        if self.document.rafraichis():
            self.a_repeindre = True
        self.annonce_titre()
        if self.document.url != self.url:
            self.url = self.document.url
            self.dis(protocole.URL_CHANGED,
                     {"contexte": self.contexte, "url": self.url})
        if self.a_repeindre:
            self.peint()

    # --- Peinture -------------------------------------------------------------

    def peint(self):
        """Joue la liste d'affichage, publie la trame, et le dit.

        L'ordre des trois est ce qui empeche l'autre cote de voir une trame a
        moitie ecrite : ecrire les pixels, publier la generation, envoyer le
        message. Le navigateur ne lit qu'apres le message, donc apres tout ce
        qui precede.
        """
        if self.surface is None or self.document is None:
            return
        rasterise = getattr(bo, "rasterise", None)
        if rasterise is None:
            self.dis(protocole.ERROR,
                     {"raison": "peinture", "detail": "hote sans rasteriseur"})
            return
        operations = self.document.liste_affichage(
            self.defilement, self.surface.largeur, self.surface.hauteur)
        octets = rasterise(operations, self.surface.largeur,
                           self.surface.hauteur)
        if not octets:
            return
        octets = bytes(octets)
        attendu = self.surface.octets_tampon
        if len(octets) < attendu:
            octets = octets + b"\x00" * (attendu - len(octets))
        elif len(octets) > attendu:
            octets = octets[:attendu]

        suivant = 1 - self.tampon
        self.surface.ecris(suivant, octets)
        self.generation += 1
        self.surface.publie(suivant, self.generation)
        self.tampon = suivant
        self.a_repeindre = False
        self.dis(protocole.FRAME_READY,
                 {"contexte": self.contexte, "generation": self.generation,
                  "tampon": suivant, "largeur": self.surface.largeur,
                  "hauteur": self.surface.hauteur,
                  "horodatage": time.monotonic()})


def sers(prise):
    """Point d'entree du processus enfant. Rend le code de sortie."""
    renderer = Renderer(prise)
    try:
        return renderer.sers()
    finally:
        if renderer.document is not None:
            try:
                renderer.document.ferme_document()
            except Exception:  # noqa: BLE001
                pass
        if renderer.surface is not None:
            renderer.surface.ferme()
