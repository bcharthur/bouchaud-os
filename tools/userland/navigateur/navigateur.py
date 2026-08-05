"""Navigateur de Bouchaud OS : interface, navigation, evenements.

Programme autonome, lance par l'hote (`hote.cpp`). Il ne connait de Qt que le
module `bo` : mesurer du texte, rendre une liste d'affichage, recevoir des
touches et des clics. Tout le reste — chrome, historique, mise en page — est ici
ou dans `moteur/`.

    exec /bo-navigateur                    page d'accueil
    exec /bo-navigateur https://…          une adresse
"""

import sys
import traceback

import bo

from moteur import Document, lecteur_youtube, reseau

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
K_F5 = 0x01000034
K_L, K_Q, K_R = 0x4C, 0x51, 0x52


class Onglet:
    def __init__(self):
        self.historique = []
        self.position = -1
        self.document = None
        self.defilement = 0.0

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

    def bat(self):
        """Battement : minuteries de la page, et remise en page si besoin.

        C'est ce qui fait qu'un `setTimeout` qui modifie le DOM se voit
        reellement a l'ecran, et que `location.href = …` navigue.
        """
        document = self.onglet.document
        if document is None:
            return
        if document.rafraichis():
            bo.redessiner()
        demande = document.navigation_demandee
        if isinstance(demande, tuple):
            pas = demande[1]
            if pas < 0:
                self.recule()
            elif pas > 0:
                self.avance()
        elif demande:
            self.ouvre(demande)

    def ouvre(self, url, empiler=True):
        url = reseau.normalise(url, self.onglet.url or None)
        if not url:
            return
        self.etat = "Chargement de %s…" % url
        self.chargement = url
        bo.redessiner()
        bo.traiter_evenements()
        try:
            document = Document(reseau.charge(url), self.largeur_vue,
                                journal=self._journal_js,
                                hauteur_fenetre=self.hauteur_vue)
        except Exception as e:
            traceback.print_exc(file=sys.stdout)
            self.etat = "Erreur interne : %s" % e
            self.chargement = None
            return
        self.chargement = None
        # La page precedente rend son interprete : sans cela, ses minuteries
        # continueraient de tourner derriere celle qu'on vient d'ouvrir.
        if self.onglet.document is not None and self.onglet.document is not document:
            self.onglet.document.ferme()
        self.onglet.document = document
        self.onglet.defilement = 0.0
        if empiler:
            self.onglet.empile(document.url)
        else:
            self.onglet.historique[self.onglet.position] = document.url
        self.saisie = document.url
        self.curseur_saisie = len(self.saisie)
        bo.titre("%s — Navigateur" % document.titre)
        if document.erreur:
            self.etat = "Echec : %s" % document.erreur
        else:
            self.etat = "%s — %d px de haut%s" % (
                document.titre, int(document.hauteur),
                "" if document.url.startswith("https://") else " — connexion en clair")

    def recule(self):
        if self.onglet.peut_reculer():
            self.onglet.position -= 1
            self._recharge_position()

    def avance(self):
        if self.onglet.peut_avancer():
            self.onglet.position += 1
            self._recharge_position()

    def _recharge_position(self):
        url = self.onglet.historique[self.onglet.position]
        document = Document(reseau.charge(url), self.largeur_vue,
                            hauteur_fenetre=self.hauteur_vue)
        self.onglet.document = document
        self.onglet.defilement = 0.0
        self.saisie = url
        self.curseur_saisie = len(self.saisie)
        bo.titre("%s — Navigateur" % document.titre)
        self.etat = document.titre

    # --- Defilement ---------------------------------------------------------

    def defile(self, pixels):
        document = self.onglet.document
        if not document:
            return
        maximum = max(0.0, document.hauteur - self.hauteur_vue + 40)
        self.onglet.defilement = min(maximum, max(0.0, self.onglet.defilement + pixels))

    # --- Peinture -----------------------------------------------------------

    def peint(self, largeur, hauteur):
        # La hauteur compte autant que la largeur : les `@media` et les
        # unites `vh` s'y rapportent, une fenetre seulement raccourcie peut
        # donc changer la page.
        remise_en_page = largeur != self.largeur or hauteur != self.hauteur
        self.largeur, self.hauteur = largeur, hauteur
        document = self.onglet.document
        if document and remise_en_page:
            document.remet_en_page(self.largeur_vue, self.hauteur_vue)

        liste = [("rect", 0, 0, largeur, hauteur, FOND_PAGE)]

        if document:
            liste.append(("clip", 0, HAUTEUR_CHROME, largeur, self.hauteur_vue))
            decalage = self.onglet.defilement - HAUTEUR_CHROME
            liste.extend(document.liste_affichage(decalage, self.largeur_vue,
                                                  self.hauteur_vue + HAUTEUR_CHROME))
            liste.append(("declip",))
            self._barre_defilement(liste, document)

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

    def _barre_defilement(self, liste, document):
        hauteur_vue = self.hauteur_vue
        if document.hauteur <= hauteur_vue:
            return
        piste_h = hauteur_vue
        pouce_h = max(30.0, piste_h * hauteur_vue / document.hauteur)
        maximum = max(1.0, document.hauteur - hauteur_vue + 40)
        position = (piste_h - pouce_h) * min(1.0, self.onglet.defilement / maximum)
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
                self.ouvre(self.saisie)
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
        if code == K_F5:
            if self.onglet.url:
                self.ouvre(self.onglet.url, empiler=False)
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
            self.onglet.defilement = 0.0
        elif code == K_FIN:
            self.defile(1e9)
        elif code == K_GAUCHE:
            self.recule()
        elif code == K_DROITE:
            self.avance()
        elif texte in ("/", ":"):
            self.champ_actif = True
            self.saisie = ""
            self.curseur_saisie = 0

    def clic(self, x, y):
        if y < HAUTEUR_CHROME:
            for nom, bx, by, bl, bh in self._boutons():
                if bx <= x <= bx + bl and by <= y <= by + bh:
                    self.champ_actif = False
                    if nom == "reculer":
                        self.recule()
                    elif nom == "avancer":
                        self.avance()
                    elif nom == "recharger" and self.onglet.url:
                        self.ouvre(self.onglet.url, empiler=False)
                    elif nom == "accueil":
                        self.ouvre("bo:accueil")
                    return
            cx, cy, cl, ch = self._champ()
            if cx <= x <= cx + cl and cy <= y <= cy + ch:
                self.champ_actif = True
                self.curseur_saisie = len(self.saisie)
                return
            self.champ_actif = False
            return

        self.champ_actif = False
        document = self.onglet.document
        if not document:
            return

        page_y = y - HAUTEUR_CHROME + self.onglet.defilement
        # Le script de la page voit le clic en premier, et peut l'annuler : un
        # `preventDefault()` sur un lien est la facon dont la moitie des sites
        # detournent la navigation.
        cible = document.element_a(x, page_y)
        if cible is not None and not document.evenement_js(
                cible, "click", {"clientX": x, "clientY": y - HAUTEUR_CHROME,
                                 "pageX": x, "pageY": page_y}):
            bo.redessiner()
            return
        self.bat()

        lien = document.lien_a(x, page_y)
        if lien:
            self.ouvre(lien)

    def survole(self, x, y):
        document = self.onglet.document
        if not document or y < HAUTEUR_CHROME:
            if self.survol:
                self.survol = None
                bo.redessiner()
            return
        lien = document.lien_a(x, y - HAUTEUR_CHROME + self.onglet.defilement)
        nouveau = ("→ " + lien) if lien else None
        if nouveau != self.survol:
            self.survol = nouveau
            bo.redessiner()

    def molette(self, cran):
        # Qt compte en huitiemes de degre ; un cran vaut 120.
        self.defile(-cran / 120.0 * 90.0)


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
    bo.ouvrir("Navigateur — Bouchaud OS")
    depart = sys.argv[1] if len(sys.argv) > 1 else "bo:accueil"
    try:
        _navigateur.ouvre(depart or "bo:accueil")
    except Exception:
        traceback.print_exc(file=sys.stdout)
    return bo.boucle()


if __name__ == "__main__":
    sys.exit(main())
