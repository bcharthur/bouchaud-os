"""Navigateur de Bouchaud OS : interface, navigation, evenements.

Programme autonome, lance par l'hote (`hote.cpp`). Il ne connait de Qt que le
module `bo` : mesurer du texte, rendre une liste d'affichage, recevoir des
touches et des clics. Tout le reste — chrome, historique, mise en page — est ici
ou dans `moteur/`.

    exec /bo-navigateur                    page d'accueil
    exec /bo-navigateur https://…          une adresse

## Ou vit la page

**Dans un autre processus.** Ce fichier tient la fenetre, le chrome et
l'historique ; le document, la cascade, le JavaScript et la peinture vivent dans
un renderer que `moteur/superviseur.py` a forke, et qui communique par un canal
de controle et une surface partagee.

Ce n'a pas toujours ete le cas : le chrome construisait son `Document` lui-meme
et lisait le DOM a portee d'attribut. Le processus separe existait mais n'etait
exerce que par les epreuves — l'architecture affichee n'etait pas
l'architecture executee. `moteur/vue.py` est ce qui a recolle les deux, et
explique ce qu'il a fallu deplacer pour cela.

L'ancien chemin reste atteignable :

    BO_BROWSER_INPROCESS=1 /bo-navigateur …

C'est un outil de comparaison, pas une option. Quand une page se comporte mal,
il tranche la question « est-ce le moteur, ou la separation ? ».
"""

import sys
import time
import traceback

import bo

from moteur import distant, lecteur_youtube, reseau, vue as mod_vue

# --- Constantes visuelles ----------------------------------------------------

HAUTEUR_CHROME = 44
HAUTEUR_ETAT = 22
MARGE = 10

FOND_CHROME = 0xFF16213E
FOND_CHAMP = 0xFF0E1729
FOND_CHAMP_ACTIF = 0xFF1B2B4D
TEXTE_CHROME = 0xFFDCE6F7
TEXTE_ESTOMPE = 0xFF7E93B8
ACCENT = 0xFF4A9EFF
FOND_ETAT = 0xFF10182B
FOND_PAGE = 0xFFFFFFFF

# Codes de touches Qt utilises (Qt::Key_*).
K_RETOUR, K_ENTREE = 0x01000000 + 4, 0x01000000 + 5
K_ECHAP = 0x01000000
K_EFFACE = 0x01000003
K_SUPPR = 0x01000007
K_GAUCHE, K_HAUT, K_DROITE, K_BAS = 0x01000012, 0x01000013, 0x01000014, 0x01000015
K_PAGE_HAUT, K_PAGE_BAS = 0x01000016, 0x01000017
K_DEBUT, K_FIN = 0x01000010, 0x01000011
K_F2 = 0x01000031
K_F5 = 0x01000034
K_L, K_Q, K_R = 0x4C, 0x51, 0x52
K_TAB, K_ESPACE = 0x01000001, 0x20
MAJ = 0x02000000

# Ce que la page appelle une touche. Le vocabulaire est celui de
# `KeyboardEvent.key` : la page le verra tel quel, et entretenir deux noms pour
# la meme touche aurait garanti qu'ils divergent.
_TOUCHES_WEB = {
    K_RETOUR: "Enter", K_ENTREE: "Enter", K_ECHAP: "Escape",
    K_EFFACE: "Backspace", K_SUPPR: "Delete", K_TAB: "Tab",
    K_GAUCHE: "ArrowLeft", K_DROITE: "ArrowRight",
    K_HAUT: "ArrowUp", K_BAS: "ArrowDown",
    K_DEBUT: "Home", K_FIN: "End",
    K_PAGE_HAUT: "PageUp", K_PAGE_BAS: "PageDown",
}


def _touche_web(code, texte):
    """Le nom Web d'une touche Qt, ou `None` si elle ne concerne pas la page."""
    nom = _TOUCHES_WEB.get(code)
    if nom is not None:
        return nom
    if texte and texte.isprintable():
        return texte
    return None


class Onglet:
    def __init__(self):
        self.historique = []
        self.position = -1
        # La page. Locale ou distante selon `moteur/vue.py` — le chrome ne sait
        # pas laquelle, et c'est le point.
        self.vue = None
        # Rendu distant : la page n'est plus un arbre mais une image, rendue
        # par un vrai navigateur sur l'hote. Voir `moteur/distant.py`.
        self.distant = None
        self.image_distante = None

    @property
    def url(self):
        return self.historique[self.position] if self.position >= 0 else ""

    def empile(self, url):
        del self.historique[self.position + 1:]
        self.historique.append(url)
        self.position = len(self.historique) - 1

    def peut_reculer(self):
        return self.position > 0

    def peut_avancer(self):
        return self.position + 1 < len(self.historique)


class Navigateur:
    def __init__(self):
        self.onglet = Onglet()
        self.largeur = 1280
        self.hauteur = 720
        self.saisie = ""
        self.champ_actif = False
        self.curseur_saisie = 0
        self.etat = "Pret."
        self.survol = None
        self.chargement = None  # URL en cours de chargement
        self.chargement_debut = None
        self._chargement_seconde = -1
        self._ouverture_differee = None
        # La vue est creee **tot**, avant que le chrome n'ait travaille. Ce
        # n'est pas un detail d'ordonnancement : `superviseur.BUDGET_AS` est un
        # budget au-dessus de ce dont l'enfant herite, et un renderer forke
        # apres une longue session naitrait au ras de son plafond. Un zygote se
        # forke tot.
        self.onglet.vue = mod_vue.ouvre_vue(
            self.largeur_vue, self.hauteur_vue, journal=self._journal_js)
        self.etat = ("Pret. (moteur en-processus)" if self.onglet.vue.genre == "local"
                     else "Pret.")

    # --- Geometrie ----------------------------------------------------------

    @property
    def hauteur_vue(self):
        return max(1, self.hauteur - HAUTEUR_CHROME - HAUTEUR_ETAT)

    @property
    def largeur_vue(self):
        return max(1, self.largeur)

    def _boutons(self):
        """Rectangles des boutons du chrome, dans l'ordre d'affichage."""
        y, h = 7, HAUTEUR_CHROME - 14
        return [
            ("reculer", MARGE, y, 30, h),
            ("avancer", MARGE + 34, y, 30, h),
            ("recharger", MARGE + 68, y, 30, h),
            ("accueil", MARGE + 102, y, 30, h),
        ]

    def _champ(self):
        gauche = MARGE + 140
        return (gauche, 7, self.largeur - gauche - MARGE, HAUTEUR_CHROME - 14)

    # --- Navigation ---------------------------------------------------------

    def _journal_js(self, niveau, texte):
        """`console.log` d'une page : sur la sortie serie, pas a l'ecran.

        Une page ne doit pas pouvoir ecrire dans l'interface du navigateur — et
        le journal reste lisible pendant un demarrage sous emulation.
        """
        print("[js:%s] %s" % (niveau, texte), flush=True)

    def differe_ouverture(self, url, empiler=True):
        """Joue la navigation au premier battement apres QApplication::exec()."""
        self._ouverture_differee = (url, bool(empiler))
        self.etat = "Initialisation du moteur de rendu..."
        self.survol = None
        bo.redessiner()

    def _termine_chargement(self):
        """La premiere trame exploitable devient la barriere de fin de chargement."""
        if self.chargement is None:
            return
        vue = self.onglet.vue
        arrivee = ((vue.url if vue is not None else "") or self.chargement)
        self.chargement = None
        self.chargement_debut = None
        self._chargement_seconde = -1
        if not self.champ_actif:
            self.saisie = arrivee
            self.curseur_saisie = len(self.saisie)
        titre = (vue.titre if vue is not None else "") or arrivee
        bo.titre("%s — Navigateur" % titre)
        if vue is not None and vue.crashee:
            self.etat = "Page crashee : %s — F5 pour recharger" % (
                vue.erreur or "renderer mort")
        elif vue is not None and vue.erreur:
            self.etat = "Echec : %s" % vue.erreur
        else:
            hauteur = int(vue.hauteur_document) if vue is not None else 0
            self.etat = "%s — %d px de haut%s" % (
                titre, hauteur,
                "" if arrivee.startswith("https://") else " — connexion en clair")

    # --- Evenements de la page ------------------------------------------------

    def applique(self, evenements):
        """Reagit a ce que la vue a dit. Le seul endroit qui le fasse.

        Les navigations ne sont **pas** appliquees par la vue : elles remontent
        ici, ou l'historique existe. Une page qui se rendrait elle-meme ailleurs
        laisserait un bouton « reculer » qui ment.
        """
        redessine = False
        navigation = None
        historique = None
        for nom, charge in evenements or ():
            if nom == "TRAME":
                if self.chargement is not None:
                    self._termine_chargement()
                redessine = True
            elif nom == "TITRE":
                bo.titre("%s — Navigateur" % (charge.get("titre") or ""))
                redessine = True
            elif nom == "URL":
                adresse = charge.get("url") or ""
                if adresse and adresse != self.onglet.url:
                    self.onglet.historique[self.onglet.position] = adresse
                if not self.champ_actif:
                    self.saisie = adresse
                    self.curseur_saisie = len(self.saisie)
                redessine = True
            elif nom == "CURSEUR":
                lien = charge.get("lien")
                nouveau = ("→ " + lien) if lien else None
                if nouveau != self.survol:
                    self.survol = nouveau
                    redessine = True
            elif nom == "NAVIGUE":
                navigation = charge.get("url")
            elif nom == "HISTORIQUE":
                historique = charge.get("pas")
            elif nom == "CRASH":
                self.chargement = None
                self.chargement_debut = None
                self._chargement_seconde = -1
                self.etat = "Page crashee : %s — F5 pour recharger" % (
                    charge.get("raison") or "renderer mort")
                self.survol = None
                redessine = True
            elif nom == "JOURNAL":
                self._journal_js(charge.get("niveau", "log"),
                                 charge.get("texte", ""))
        if redessine:
            bo.redessiner()
        # La navigation en dernier : elle remplace la vue, et jouer les autres
        # evenements apres l'aurait fait sur une page qui n'existe plus.
        if historique:
            if historique < 0:
                self.recule()
            elif historique > 0:
                self.avance()
        elif navigation:
            self.commence_ouverture(navigation)

    def bat(self):
        """Tour court de la boucle produit : aucune attente reseau ici."""
        if self.onglet.distant is not None:
            self.bat_distant()
        elif self.onglet.vue is not None:
            self.applique(self.onglet.vue.bat())

        if self._ouverture_differee is not None:
            vue = self.onglet.vue
            pret = (vue is None or vue.genre != "renderer"
                    or getattr(vue, "pret", False) or vue.crashee)
            if pret:
                url, empiler = self._ouverture_differee
                self._ouverture_differee = None
                self.commence_ouverture(url, empiler=empiler)

        if self.chargement is not None and self.chargement_debut is not None:
            secondes = int(max(0.0, time.monotonic() - self.chargement_debut))
            if secondes != self._chargement_seconde:
                self._chargement_seconde = secondes
                self.etat = "Chargement de %s... %d s" % (self.chargement, secondes)
                bo.redessiner()

    def _ouvre_impl(self, url, empiler=True, attendre=False):
        """Implementation commune aux contrats synchrone et asynchrone."""
        brut = (url or "").strip()
        if brut.startswith("distant:"):
            return self.ouvre_distant(
                reseau.normalise(brut[len("distant:"):], self.onglet.url or None))
        url = reseau.normalise(url, self.onglet.url or None)
        if not url:
            return False

        self.etat = "Chargement de %s..." % url
        self.chargement = url
        self.chargement_debut = time.monotonic()
        self._chargement_seconde = 0
        self.survol = None
        bo.redessiner()

        vue = self.onglet.vue
        if vue is None:
            vue = self.onglet.vue = mod_vue.ouvre_vue(
                self.largeur_vue, self.hauteur_vue, journal=self._journal_js)
        try:
            vue.redimensionne(self.largeur_vue, self.hauteur_vue)
            if (not attendre and vue.genre == "renderer"
                    and hasattr(vue, "commence_ouverture")):
                lancee = vue.commence_ouverture(url)
            else:
                lancee = vue.ouvre(url)
        except Exception as e:
            traceback.print_exc(file=sys.stdout)
            self.etat = "Erreur interne : %s" % e
            self.chargement = None
            self.chargement_debut = None
            return False

        arrivee = vue.url or url
        if empiler:
            self.onglet.empile(arrivee)
        elif self.onglet.position >= 0:
            self.onglet.historique[self.onglet.position] = arrivee
        else:
            self.onglet.empile(arrivee)
        self.saisie = arrivee
        self.curseur_saisie = len(self.saisie)
        bo.titre("%s — Navigateur" % (vue.titre or arrivee))

        # Dans le chemin synchrone, VueRenderer.ouvre() a deja recu sa trame :
        # le chrome doit donc exposer titre/etat final avant de rendre la main.
        # Dans le chemin Qt, seule la premiere TRAME appelle cette fonction.
        if attendre or vue.genre == "local" or lancee is False or vue.crashee:
            self._termine_chargement()
        return bool(lancee is not False)

    def commence_ouverture(self, url, empiler=True):
        """Navigation de l'UI : aucune attente de page dans le fil Qt."""
        return self._ouvre_impl(url, empiler=empiler, attendre=False)

    def ouvre(self, url, empiler=True):
        """Contrat historique synchrone, conserve pour tests et outils."""
        return self._ouvre_impl(url, empiler=empiler, attendre=True)

    # --- Rendu distant --------------------------------------------------------

    def ouvre_distant(self, url):
        """Ouvre une adresse dans le Chromium deporte. Rend `True` si l'image vient.

        C'est la seule facon d'afficher ce qu'aucun moteur ecrit ici ne rendra :
        une application qui compile son interface, un lecteur qui pousse ses
        segments, une page qui dessine en WebGL. La page cesse d'etre un arbre
        pour devenir une image — on ne peut plus la selectionner ni la chercher,
        et il faut un hote qui tourne. C'est le marche, et il est explicite.
        """
        session = self.onglet.distant
        if session is None:
            session = distant.Session(journal=self._journal_js)
            self.onglet.distant = session

        self.etat = "Rendu distant : %s…" % url
        bo.redessiner()
        bo.traiter_evenements()

        if not session.ouvre(url, self.largeur_vue, self.hauteur_vue):
            self.etat = ("Service de rendu injoignable (%s) — %s"
                         % (session.service, session.erreur or "sans reponse"))
            self.onglet.distant = None
            return False

        self._prend_image()
        session.etat()
        if self.onglet.vue is not None:
            self.onglet.vue.ferme()
            self.onglet.vue = None
        self.onglet.empile(session.url or url)
        self.saisie = session.url or url
        self.curseur_saisie = len(self.saisie)
        bo.titre("%s — Navigateur (distant)" % (session.titre or url))
        self.etat = "%s — rendu distant" % (session.titre or url)
        return True

    def bascule_distant(self):
        """Passe la page courante du moteur natif au rendu distant, et retour.

        Le choix reste a l'utilisateur, et c'est voulu : le moteur natif donne
        une page vivante — texte selectionnable, defilement instantane, aucune
        dependance — la ou le rendu distant donne une image de tout le web. Ni
        l'un ni l'autre ne remplace l'autre.
        """
        url = self.onglet.url
        if self.onglet.distant is not None:
            self.onglet.distant = None
            self.onglet.image_distante = None
            if url:
                self.commence_ouverture(url, empiler=False)
            else:
                self.etat = "Retour au moteur natif."
            return
        if not url:
            self.etat = "Rien a rendre."
            return
        self.ouvre_distant(url)

    def _prend_image(self):
        """Fait decoder la derniere image recue par l'hote Qt."""
        session = self.onglet.distant
        if session is None or not session.octets:
            self.onglet.image_distante = None
            return
        try:
            self.onglet.image_distante = bo.image(session.octets)
        except Exception as e:  # noqa: BLE001
            self._journal_js("warn", "image distante illisible : %s" % e)
            self.onglet.image_distante = None

    def bat_distant(self):
        """Redemande l'image si la page distante a bouge.

        Le service ne repond que lorsqu'il a autre chose a montrer : une page
        immobile ne coute rien, une video se rafraichit d'elle-meme.
        """
        session = self.onglet.distant
        if session is None:
            return
        if session.rafraichis():
            self._prend_image()
            bo.redessiner()

    def _agit_distant(self, action):
        """Joue une action sur la page distante et reprend l'image."""
        if action():
            self._prend_image()
            self.onglet.distant.etat()
            self.saisie = self.onglet.distant.url or self.saisie
            bo.redessiner()

    def recule(self):
        if self.onglet.distant is not None:
            self._agit_distant(self.onglet.distant.recule)
            return
        if self.onglet.peut_reculer():
            self.onglet.position -= 1
            self._recharge_position()

    def avance(self):
        if self.onglet.distant is not None:
            self._agit_distant(self.onglet.distant.avance)
            return
        if self.onglet.peut_avancer():
            self.onglet.position += 1
            self._recharge_position()

    def _recharge_position(self):
        """Rejoue l'entree courante sans bloquer l'event-loop."""
        url = self.onglet.historique[self.onglet.position]
        self.commence_ouverture(url, empiler=False)

    # --- Defilement ---------------------------------------------------------

    def defile(self, pixels):
        if self.onglet.distant is not None:
            self._agit_distant(lambda: self.onglet.distant.defile(int(pixels)))
            return
        vue = self.onglet.vue
        if vue is None:
            return
        vue.defile(pixels)
        bo.redessiner()

    # --- Peinture -----------------------------------------------------------

    def peint(self, largeur, hauteur):
        # La hauteur compte autant que la largeur : les `@media` et les
        # unites `vh` s'y rapportent, une fenetre seulement raccourcie peut
        # donc changer la page. Le redimensionnement part a la vue, qui le
        # transmet au renderer — le chrome ne remet plus rien en page lui-meme.
        remise_en_page = largeur != self.largeur or hauteur != self.hauteur
        self.largeur, self.hauteur = largeur, hauteur
        vue = self.onglet.vue
        if vue is not None and remise_en_page:
            vue.redimensionne(self.largeur_vue, self.hauteur_vue)

        liste = [("rect", 0, 0, largeur, hauteur, FOND_PAGE)]

        # Rendu distant : il n'y a pas d'arbre a peindre, seulement l'image que
        # le Chromium de l'hote a produite. Elle est posee telle quelle, calee
        # en haut de la zone de contenu.
        if self.onglet.distant is not None:
            image = self.onglet.image_distante
            liste.append(("clip", 0, HAUTEUR_CHROME, largeur, self.hauteur_vue))
            if image is not None:
                identifiant, image_l, image_h = image
                liste.append(("image", 0, HAUTEUR_CHROME, image_l, image_h,
                              identifiant))
            liste.append(("declip",))
            self._chrome(liste)
            self._barre_etat(liste)
            return liste

        if vue is not None:
            liste.append(("clip", 0, HAUTEUR_CHROME, largeur, self.hauteur_vue))
            # `VueLocale` pose une liste d'affichage, `VueRenderer` pose une
            # image : les deux savent se peindre dans la zone de contenu, et le
            # chrome n'a pas a savoir laquelle il tient.
            vue.peint(liste, HAUTEUR_CHROME)
            liste.append(("declip",))
            self._barre_defilement(liste, vue)

        self._chrome(liste)
        self._barre_etat(liste)
        return liste

    def _chrome(self, liste):
        liste.append(("rect", 0, 0, self.largeur, HAUTEUR_CHROME, FOND_CHROME))
        liste.append(("rect", 0, HAUTEUR_CHROME - 1, self.largeur, 1, 0xFF2A4580))

        etiquettes = {"reculer": "←", "avancer": "→", "recharger": "⟳", "accueil": "⌂"}
        actifs = {
            "reculer": self.onglet.peut_reculer(),
            "avancer": self.onglet.peut_avancer(),
            "recharger": True,
            "accueil": True,
        }
        for nom, x, y, l, h in self._boutons():
            liste.append(("rond", x, y, l, h, 6, FOND_CHAMP))
            teinte = TEXTE_CHROME if actifs[nom] else TEXTE_ESTOMPE
            texte = etiquettes[nom]
            largeur_texte = bo.largeur_texte(texte, 15, False, False)
            liste.append(("texte", x + (l - largeur_texte) / 2, y + (h - 18) / 2,
                          texte, teinte, 15, False, False, False, False))

        x, y, l, h = self._champ()
        liste.append(("rond", x, y, l, h, 8,
                      FOND_CHAMP_ACTIF if self.champ_actif else FOND_CHAMP))
        if self.champ_actif:
            liste.append(("rond", x, y + h - 2, l, 2, 1, ACCENT))
        texte = self.saisie or "Adresse ou recherche…"
        teinte = TEXTE_CHROME if self.saisie else TEXTE_ESTOMPE
        liste.append(("clip", x + 10, y, l - 20, h))
        liste.append(("texte", x + 10, y + (h - 18) / 2, texte, teinte, 14,
                      False, False, False, False))
        if self.champ_actif:
            avant = self.saisie[:self.curseur_saisie]
            position = x + 10 + bo.largeur_texte(avant, 14, False, False)
            liste.append(("rect", position, y + 6, 1.5, h - 12, ACCENT))
        liste.append(("declip",))

    def _barre_etat(self, liste):
        y = self.hauteur - HAUTEUR_ETAT
        liste.append(("rect", 0, y, self.largeur, HAUTEUR_ETAT, FOND_ETAT))
        message = self.survol or self.etat
        liste.append(("texte", MARGE, y + 4, message[:200], TEXTE_ESTOMPE, 12,
                      False, False, False, False))

    def _barre_defilement(self, liste, vue):
        hauteur_vue = self.hauteur_vue
        hauteur_document = vue.hauteur_document
        if hauteur_document <= hauteur_vue:
            return
        piste_h = hauteur_vue
        pouce_h = max(30.0, piste_h * hauteur_vue / hauteur_document)
        maximum = max(1.0, hauteur_document - hauteur_vue + 40)
        position = (piste_h - pouce_h) * min(1.0, vue.defilement / maximum)
        x = self.largeur - 8
        liste.append(("rect", x, HAUTEUR_CHROME, 6, piste_h, 0xFFEFF1F5))
        liste.append(("rond", x, HAUTEUR_CHROME + position, 6, pouce_h, 3, 0xFFB9C2D0))

    # --- Evenements ---------------------------------------------------------

    def touche(self, code, texte, modificateurs=0):
        CTRL = 0x04000000
        ctrl = bool(modificateurs & CTRL)
        ctrl_l = ctrl and code == K_L
        if self.champ_actif:
            if code in (K_RETOUR, K_ENTREE):
                self.champ_actif = False
                self.commence_ouverture(self.saisie)
                return
            if code == K_ECHAP:
                self.champ_actif = False
                self.saisie = self.onglet.url
                return
            if code == K_EFFACE:
                if self.curseur_saisie > 0:
                    self.saisie = (self.saisie[:self.curseur_saisie - 1]
                                   + self.saisie[self.curseur_saisie:])
                    self.curseur_saisie -= 1
                return
            if code == K_SUPPR:
                self.saisie = (self.saisie[:self.curseur_saisie]
                               + self.saisie[self.curseur_saisie + 1:])
                return
            if code == K_GAUCHE:
                self.curseur_saisie = max(0, self.curseur_saisie - 1)
                return
            if code == K_DROITE:
                self.curseur_saisie = min(len(self.saisie), self.curseur_saisie + 1)
                return
            if code == K_DEBUT:
                self.curseur_saisie = 0
                return
            if code == K_FIN:
                self.curseur_saisie = len(self.saisie)
                return
            if texte and texte.isprintable():
                self.saisie = (self.saisie[:self.curseur_saisie] + texte
                               + self.saisie[self.curseur_saisie:])
                self.curseur_saisie += len(texte)
                return
            return

        if ctrl_l:
            self.champ_actif = True
            self.curseur_saisie = len(self.saisie)
            return

        # Un champ de la **page** a le foyer : la frappe lui appartient, pas au
        # chrome. Sans cette bifurcation, `Bas` faisait defiler la page pendant
        # qu'on essayait d'ecrire, et aucune lettre n'atteignait le formulaire.
        #
        # Le chrome ne lit plus le DOM pour le savoir : la vue tient le bit, et
        # le renderer le lui envoie (`FOCUS_CHANGED`) chaque fois qu'il change.
        # C'est le seul etat du document dont le chrome ait besoin.
        vue = self.onglet.vue
        if vue is not None and self.onglet.distant is None and vue.foyer:
            nom = _touche_web(code, texte)
            if nom is not None:
                self.applique(vue.touche(nom, texte or "",
                                         maj=bool(modificateurs & MAJ), ctrl=ctrl))
                bo.redessiner()
                return
        if code == K_F5:
            if self.onglet.url:
                self.commence_ouverture(self.onglet.url, empiler=False)
            return
        if ctrl and code == K_Q:
            bo.quitter()
            return
        if code == K_BAS:
            self.defile(60)
        elif code == K_HAUT:
            self.defile(-60)
        elif code == K_PAGE_BAS:
            self.defile(self.hauteur_vue * 0.9)
        elif code == K_PAGE_HAUT:
            self.defile(-self.hauteur_vue * 0.9)
        elif code == K_DEBUT:
            if self.onglet.vue is not None:
                self.onglet.vue.va_en_haut()
                bo.redessiner()
        elif code == K_FIN:
            self.defile(1e9)
        elif code == K_GAUCHE:
            self.recule()
        elif code == K_DROITE:
            self.avance()
        elif code == K_F2:
            self.bascule_distant()
        elif self.onglet.distant is not None:
            # En rendu distant, ce qui n'est pas un raccourci du navigateur
            # appartient a la page : elle a ses propres champs et ses propres
            # raccourcis, et les garder ici les lui volerait.
            nomme = distant.touche_pour(_nom_de_touche(code))
            if nomme:
                self._agit_distant(lambda: self.onglet.distant.touche(nomme))
            elif texte and texte.isprintable():
                self._agit_distant(lambda: self.onglet.distant.saisis(texte))
        elif texte in ("/", ":"):
            self.champ_actif = True
            self.saisie = ""
            self.curseur_saisie = 0

    def clic(self, x, y):
        if y >= HAUTEUR_CHROME and self.onglet.distant is not None:
            self.champ_actif = False
            page_y = y - HAUTEUR_CHROME
            self._agit_distant(lambda: self.onglet.distant.clic(x, page_y))
            return
        if y < HAUTEUR_CHROME:
            for nom, bx, by, bl, bh in self._boutons():
                if bx <= x <= bx + bl and by <= y <= by + bh:
                    self.champ_actif = False
                    if nom == "reculer":
                        self.recule()
                    elif nom == "avancer":
                        self.avance()
                    elif nom == "recharger" and self.onglet.url:
                        if self.onglet.distant is not None:
                            self._agit_distant(self.onglet.distant.recharge)
                        else:
                            self.commence_ouverture(self.onglet.url, empiler=False)
                    elif nom == "accueil":
                        self.commence_ouverture("bo:accueil")
                    return
            cx, cy, cl, ch = self._champ()
            if cx <= x <= cx + cl and cy <= y <= cy + ch:
                self.champ_actif = True
                self.curseur_saisie = len(self.saisie)
                return
            self.champ_actif = False
            return

        self.champ_actif = False
        vue = self.onglet.vue
        if vue is None:
            return

        # Le script de la page voit le clic en premier, et peut l'annuler : un
        # `preventDefault()` sur un lien est la facon dont la moitie des sites
        # detournent la navigation.
        # La sequence complete — foyer, evenement, action par defaut, lien —
        # vit dans le moteur, du cote ou vit la page. Le chrome n'en voit que
        # le resultat : eventuellement une demande de navigation, qu'il
        # applique ou non.
        self.applique(vue.clic(x, y - HAUTEUR_CHROME))

    def survole(self, x, y):
        vue = self.onglet.vue
        if vue is None:
            return
        if y < HAUTEUR_CHROME:
            # Le pointeur a quitte la page : elle doit le savoir, sinon un
            # `:hover` reste allume sous une barre d'outils.
            self.applique(vue.souris(-1, -1))
            if self.survol:
                self.survol = None
                bo.redessiner()
            return
        # `:hover` : la page se restyle sous le pointeur. Le document ne
        # recalcule que si sa feuille parle d'interaction, donc un mouvement
        # ordinaire ne coute qu'un test de position.
        self.applique(vue.souris(x, y - HAUTEUR_CHROME))

    def molette(self, cran):
        # Qt compte en huitiemes de degre ; un cran vaut 120.
        self.defile(-cran / 120.0 * 90.0)


def _nom_de_touche(code):
    """Nom interne d'une touche speciale de Qt, ou `""`."""
    return {
        K_ENTREE: "entree", K_RETOUR: "entree", K_EFFACE: "retour",
        K_SUPPR: "suppr", K_ECHAP: "echap",
        K_HAUT: "haut", K_BAS: "bas", K_GAUCHE: "gauche", K_DROITE: "droite",
        K_PAGE_HAUT: "page_haut", K_PAGE_BAS: "page_bas",
        K_DEBUT: "debut", K_FIN: "fin",
    }.get(code, "")


_navigateur = Navigateur()


# --- Points d'entree appeles par l'hote --------------------------------------

def _peindre(largeur, hauteur):
    try:
        return _navigateur.peint(largeur, hauteur)
    except Exception:
        traceback.print_exc(file=sys.stdout)
        return [("rect", 0, 0, largeur, hauteur, 0xFFFFFFFF),
                ("texte", 20, 20, "Erreur de peinture — voir la console",
                 0xFFB3261E, 16, True, False, False, False)]


def _touche(code, texte, modificateurs=0):
    try:
        _navigateur.touche(code, texte, modificateurs)
    except Exception:
        traceback.print_exc(file=sys.stdout)
    return True


def _clic(x, y):
    try:
        _navigateur.clic(x, y)
    except Exception:
        traceback.print_exc(file=sys.stdout)
    return True


def _survol(x, y):
    try:
        _navigateur.survole(x, y)
    except Exception:
        pass
    return True


def _molette(cran):
    try:
        _navigateur.molette(cran)
    except Exception:
        traceback.print_exc(file=sys.stdout)
    return True


def _fermeture():
    # La vue ferme son renderer : sans cela, la fenetre disparaitrait en
    # laissant derriere elle un processus qui peint pour personne — jusqu'a ce
    # que `SILENCE_MAX_S` le reveille, deux minutes plus tard.
    try:
        if _navigateur.onglet.vue is not None:
            _navigateur.onglet.vue.ferme()
    except Exception:
        traceback.print_exc(file=sys.stdout)
    return True


def _tic():
    # Battement de l'hote, toutes les 16 ms. C'est par lui que vivent les
    # minuteries de la page : sans ce branchement, un `setTimeout` serait
    # enregistre et jamais echu.
    try:
        _navigateur.bat()
    except Exception:
        traceback.print_exc(file=sys.stdout)


def _detour_youtube(url):
    """Substitue une page de lecture aux adresses YouTube.

    Branche sur la couche reseau plutot qu'appele depuis `ouvre` : ainsi un lien
    YouTube clique dans une autre page, ou saisi dans la barre d'adresse, passe
    par le meme chemin.
    """
    if not lecteur_youtube.est_pris_en_charge(url):
        return None
    return lecteur_youtube.page(
        url, journal=lambda niveau, texte: print("[yt:%s] %s" % (niveau, texte),
                                                 flush=True))


def verifie(depart="bo:accueil", battements=60):
    """Demarre le navigateur, ouvre une page, et rend compte. Sans fenetre.

    ## A quoi cela sert

    A prouver, depuis Bouchaud OS lui-meme, que le binaire livre dans
    `userland.img` demarre vraiment : que Qt s'initialise, que CPython execute
    le chrome, que le renderer se forke, qu'une page se charge et qu'une trame
    est peinte. C'est le seul moyen d'obtenir ce verdict sans quelqu'un devant
    l'ecran, et donc le seul moyen de l'obtenir a chaque commit.

    ## Pourquoi sans fenetre

    `bo.ouvrir` demande un peripherique d'affichage et `bo.boucle` ne rend la
    main que lorsqu'on ferme la fenetre : ni l'un ni l'autre n'a de sens dans un
    scenario qui doit se terminer seul. On appelle donc exactement les rappels
    que Qt appellerait — `peindre`, `tic` — sur le meme objet `Navigateur`.
    C'est le chemin du produit, pas une imitation : la vue est la vraie, le
    renderer est un vrai processus, et la trame vient de la surface partagee.
    """
    lignes = []
    def note(nom, ok, detail=""):
        lignes.append((nom, bool(ok), str(detail)[:120]))

    vue = _navigateur.onglet.vue
    note("vue separee", vue is not None and vue.genre == "renderer",
         getattr(vue, "genre", None))
    pid_renderer = getattr(getattr(vue, "renderer", None), "pid", None)
    note("processus renderer", bool(pid_renderer), pid_renderer)

    try:
        _navigateur.ouvre(depart or "bo:accueil")
    except Exception as e:  # noqa: BLE001
        traceback.print_exc(file=sys.stdout)
        note("chargement", False, e)
    else:
        limite_battements = max(int(battements), 180)
        for _ in range(limite_battements):
            _tic()
            if vue and (vue.crashee or (vue.trames > 0 and vue.url)):
                break
            time.sleep(0.016)
        note("chargement", bool(vue and not vue.crashee and vue.trames > 0),
             vue.erreur if vue else "")
    note("adresse", bool(vue and vue.url), getattr(vue, "url", None))
    note("titre", bool(vue and vue.titre), getattr(vue, "titre", None))
    note("trame peinte", bool(vue and vue.trames > 0), getattr(vue, "trames", 0))

    liste = _peindre(1280, 720)
    images = [op for op in liste if op[0] == "image"]
    note("page posee en image", len(images) == 1, len(images))
    note("chrome peint", any(op[0] == "texte" for op in liste), len(liste))

    print("")
    for nom, ok, detail in lignes:
        print("  %s  %s%s" % ("ok  " if ok else "ECHEC", nom,
                              (" — " + detail) if detail else ""))
    echecs = sum(1 for _n, ok, _d in lignes if not ok)
    # La forme de cette ligne est celle que `tools/test.sh` et la CI comptent.
    print("RESULTAT : %d verification(s) en echec (%d passees)"
          % (echecs, len(lignes) - echecs))
    try:
        _fermeture()
    except Exception:  # noqa: BLE001
        pass
    return 1 if echecs else 0


def main():
    reseau.installe_detour_youtube(_detour_youtube)
    bo.enregistrer({
        "peindre": _peindre,
        "touche": _touche,
        "clic": _clic,
        "survol": _survol,
        "molette": _molette,
        "fermeture": _fermeture,
        "tic": _tic,
    })
    arguments = [a for a in sys.argv[1:] if a]

    # `--verifie` : demarrer, ouvrir, peindre, rendre compte, sortir. Voir
    # `verifie`. Le mode existe pour la CI, mais rien n'y est reserve aux
    # tests : c'est le navigateur, sans sa fenetre.
    if "--verifie" in arguments:
        arguments.remove("--verifie")
        return verifie(arguments[0] if arguments else "bo:accueil")

    bo.ouvrir("Navigateur — Bouchaud OS")
    depart = arguments[0] if arguments else "bo:accueil"
    _navigateur.differe_ouverture(depart or "bo:accueil")
    print("[bo] navigation initiale differee jusqu'a l'event-loop : %s"
          % (depart or "bo:accueil"), flush=True)
    return bo.boucle()


if __name__ == "__main__":
    sys.exit(main())
