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

## Les ressources : demandees, pas prises

Par defaut, ce processus ne va **pas** chercher ses ressources lui-meme. Il
installe un `transport.Courtier` : chaque `reseau.charge` devient un
`FETCH_REQUEST` sur le canal de controle, et c'est le navigateur qui applique la
politique, lit les temoins et ouvre la prise. Voir `transport.py` pour ce que
cela deplace exactement, et `BO_RENDERER_DIRECT=1` pour l'outil de comparaison
qui remet l'ancien chemin.
"""

import os
import resource
import socket
import time
import urllib.parse

import bo

from . import privileges, protocole, surface as mod_surface, transport

# Le renderer se coupe si le navigateur ne lui parle plus. Sans cela, un
# navigateur tue laisserait derriere lui un processus qui peint pour personne.
SILENCE_MAX_S = 120.0

# Au-dela, une ressource courtee est declaree perdue. Plus long que le delai
# reseau du navigateur (`reseau.DELAI`, 20 s) : c'est lui qui doit rendre le
# verdict d'echec, pas nous — sinon deux horloges decident de la meme chose et
# la plus courte gagne, en donnant une erreur moins precise.
DELAI_FETCH_S = 45.0


class Renderer:
    """L'etat d'un renderer : un document, une surface, une prise."""

    def __init__(self, prise, courtage=True, audit=False):
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
        self.foyer = False
        self.curseur = "fleche"
        self.lien_survole = None
        # `AUDIT` n'est pas une capacite qu'un renderer de production possede :
        # elle est accordee au `fork`, par le navigateur, et seulement aux
        # renderers d'audit. Un renderer ordinaire ne sait litteralement pas
        # repondre a la question « que possedes-tu ? ».
        self.audit = bool(audit)
        # Ce qui est arrive pendant qu'on attendait une reponse a une requete.
        # Rejoue ensuite, dans l'ordre : voir `attends_fetch`.
        self.differes = []
        self._suivant_fetch = 1
        if courtage:
            transport.installe(transport.Courtier(self.demande_ressource))

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
                # Ce qui a ete mis de cote pendant une requete passe d'abord :
                # un `TICK` arrive au milieu d'un chargement doit etre joue,
                # pas perdu, et il doit l'etre **dans l'ordre**.
                if self.differes:
                    genre, charge = self.differes.pop(0)
                else:
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
        elif genre == protocole.AUDIT:
            self.repond_audit(charge)
        elif genre in (protocole.FETCH_RESPONSE, protocole.FETCH_DATA):
            # Une reponse sans demande en cours. Rien a en faire, et surtout pas
            # a la croire : elle est jetee.
            pass

    # --- Ressources courtees --------------------------------------------------

    def demande_ressource(self, requete):
        """Envoie un `FETCH_REQUEST` et rend la charge de la reponse.

        C'est le seul chemin par lequel une ressource entre dans ce processus
        quand le courtage est actif. Rendre une erreur plutot que lever est
        deliberé : `reseau.charge` promet de ne jamais echouer par exception, et
        toute la cascade compte dessus.
        """
        identifiant = self._suivant_fetch
        self._suivant_fetch += 1
        requete = dict(requete, id=identifiant)
        try:
            self.dis(protocole.FETCH_REQUEST, requete)
            return self.attends_fetch(identifiant)
        except protocole.Fin:
            raise
        except protocole.Erreur as e:
            return {"url": requete["url"], "code": 0,
                    "erreur": "courtier : %s" % e, "type_mime": "text/plain"}

    def attends_fetch(self, identifiant):
        """Lit jusqu'a la reponse `identifiant`, en gardant le reste pour plus tard.

        Tout ce qui n'est pas cette reponse est **mis de cote**, pas traite :
        traiter un `INPUT_EVENT` ici rentrerait dans le moteur alors qu'on est
        deja au milieu d'un chargement, et une cascade reentrante est une des
        rares facons qu'a ce code de se corrompre silencieusement.
        """
        morceaux = []
        meta = None
        limite = time.monotonic() + DELAI_FETCH_S
        while True:
            if time.monotonic() > limite:
                raise protocole.Erreur("reponse %d jamais venue" % identifiant)
            try:
                genre, charge = self.canal.lis(protocole.VERS_RENDERER)
            except socket.timeout:
                raise protocole.Erreur("reponse %d : silence" % identifiant)
            charge = charge or {}
            if genre == protocole.FETCH_RESPONSE and charge.get("id") == identifiant:
                meta = charge
                if charge.get("fin"):
                    break
            elif genre == protocole.FETCH_DATA and charge.get("id") == identifiant:
                donnees = charge.get("b64")
                if donnees:
                    morceaux.append(donnees)
                if charge.get("fin"):
                    break
            elif genre == protocole.CLOSE:
                raise protocole.Fin()
            else:
                self.differes.append((genre, charge))
        if meta is None:
            raise protocole.Erreur("reponse %d sans en-tete" % identifiant)
        import base64
        octets = base64.b64decode("".join(morceaux)) if morceaux else b""
        reponse = dict(meta)
        reponse["octets"] = octets
        if not meta.get("texte_propre"):
            reponse.pop("contenu", None)
        return reponse

    # --- Audit -----------------------------------------------------------------

    def repond_audit(self, charge):
        """Dit ce que ce processus possede. **Seulement si on lui a accorde.**

        Un renderer de production ignore la demande, et c'est le comportement
        correct : la reponse est un plan detaille de ce qu'un attaquant pourrait
        atteindre depuis ici. Elle n'existe que pour les epreuves, qui creent
        leur renderer avec la capacite.
        """
        if not self.audit:
            self.dis(protocole.ERROR,
                     {"raison": "AUDIT", "detail": "capacite non accordee"})
            return
        quoi = str(charge.get("quoi", "descripteurs"))
        reponse = {"quoi": quoi, "pid": os.getpid(),
                   "transport": getattr(transport.actuel(), "nom", "direct")}
        if quoi == "descripteurs":
            reponse["descripteurs"] = privileges.inventaire()
        elif quoi == "tentatives":
            cible = charge.get("cible_reseau")
            accordes = [self.prise.fileno()]
            if self.surface is not None and self.surface.descripteur is not None:
                accordes.append(self.surface.descripteur)
            reponse["tentatives"] = privileges.tentatives(
                cible_reseau=tuple(cible) if cible else None,
                fichier=charge.get("fichier"), accordes=accordes)
            reponse["accordes"] = accordes
        elif quoi == "navigation_interdite":
            # Le renderer demande une navigation que la politique doit refuser.
            # C'est la seule tentative qui ne se joue pas ici : elle se joue
            # **de l'autre cote**, et c'est precisement ce qu'on veut prouver.
            self.dis(protocole.REQUEST_NAVIGATION,
                     {"contexte": self.contexte,
                      "url": str(charge.get("url") or "file:///etc/shadow"),
                      "provenance": self.url or "https://exemple.test/"})
            reponse["demande"] = True
        self.dis(protocole.AUDIT_RESULT, reponse)

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
            x = float(charge.get("x", 0))
            y = float(charge.get("y", 0)) + self.defilement
            if self.document.survole(x, y):
                self.a_repeindre = True
            lien = self.document.lien_a(x, y)
            forme = "pointeur" if lien else "fleche"
            # La forme **et** le lien dans le meme message. Le chrome affiche
            # l'adresse survolee dans sa barre d'etat ; la lui faire chercher
            # dans le DOM d'en face aurait ete exactement la lecture directe que
            # cette architecture existe pour supprimer.
            if forme != self.curseur or lien != self.lien_survole:
                self.curseur, self.lien_survole = forme, lien
                self.dis(protocole.CURSOR_CHANGED,
                         {"contexte": self.contexte, "forme": forme,
                          "lien": lien})
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
                self.demande_navigation(url)
            self.annonce_foyer()
        elif genre == "touche":
            self.document.frappe(str(charge.get("touche", "")),
                                 str(charge.get("texte", "")),
                                 bool(charge.get("maj")),
                                 bool(charge.get("ctrl")))
            self.a_repeindre = True
            self.annonce_foyer()
        elif genre == "defilement":
            self.defilement = max(0.0, float(charge.get("position", 0)))
            self.a_repeindre = True

    def demande_navigation(self, url):
        """Demande au navigateur d'aller quelque part. **Adresse absolue.**

        La resolution se fait ici, contre l'adresse du document. Ce n'est pas
        une commodite pour l'appelant : la politique du navigateur compare un
        **schema**, et un schema ne se lit pas dans `../ailleurs`. Envoyer
        l'adresse brute reviendrait a demander a la politique de deviner contre
        quoi resoudre, c'est-a-dire a la faire dependre d'un etat que le
        renderer lui annonce — exactement ce qu'on cherche a ne pas faire.
        """
        absolue = urllib.parse.urljoin(self.url or "", str(url)) \
            if self.url else str(url)
        self.dis(protocole.REQUEST_NAVIGATION,
                 {"contexte": self.contexte, "url": absolue,
                  "provenance": self.url})

    def annonce_foyer(self):
        """Dit au chrome si un champ de la page tient le clavier.

        Sans cela, le chrome ne saurait pas a qui donner la prochaine frappe :
        en-processus il lisait `document.foyer_actuel()`, ce qu'un chrome
        separe du DOM ne peut plus faire. C'est le seul bit du DOM dont le
        chrome ait besoin, et il vient donc par message.
        """
        foyer = self.document is not None and self.document.foyer_actuel() is not None
        if foyer != self.foyer:
            self.foyer = foyer
            self.dis(protocole.FOCUS_CHANGED,
                     {"contexte": self.contexte, "foyer": foyer})

    def bat(self):
        if self.document is None:
            return
        if self.document.rafraichis():
            self.a_repeindre = True
        self.annonce_titre()
        self.annonce_foyer()
        # `location.href = …` : le script demande, le navigateur decide. Le
        # renderer ne suit pas la demande lui-meme — c'est la meme regle que
        # pour un lien clique, et l'oublier ici aurait laisse une porte ouverte
        # que la porte de devant refuse.
        demande = self.document.navigation_demandee
        if isinstance(demande, tuple):
            self.dis(protocole.REQUEST_NAVIGATION,
                     {"contexte": self.contexte, "pas": int(demande[1]),
                      "provenance": self.url})
        elif demande:
            self.demande_navigation(demande)
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
                  # La hauteur du document et la position du defilement
                  # voyagent avec la trame : c'est ce dont le chrome a besoin
                  # pour dessiner son ascenseur, et c'est la seule autre chose
                  # qu'il lisait dans le DOM.
                  "hauteur_document": float(getattr(self.document, "hauteur", 0.0)),
                  "defilement": float(self.defilement),
                  "horodatage": time.monotonic()})


def sers(prise, courtage=True, audit=False):
    """Point d'entree du processus enfant. Rend le code de sortie."""
    renderer = Renderer(prise, courtage=courtage, audit=audit)
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
