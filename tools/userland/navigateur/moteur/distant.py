"""Client leger : afficher ce qu'un vrai Chromium a rendu ailleurs.

Le moteur natif de ce dossier affiche beaucoup de choses, et il ne les affichera
jamais toutes. Un site qui compile son interface a la volee, un lecteur qui
pousse ses segments par Media Source Extensions, une page qui dessine en WebGL —
les rendre demanderait Blink ou WebKit, c'est-a-dire des millions de lignes et
un portage de plusieurs mois.

Ce module prend l'autre chemin, celui d'Opera Mini et de Puffin : **le rendu se
fait ailleurs**, sur une machine qui a un vrai navigateur, et l'OS n'affiche que
l'image. Ce qu'on y gagne n'est pas un progres partiel — c'est la totalite du
web, y compris ce qu'aucun moteur ecrit ici ne rendra : les applications, les
videos protegees, WebGL, les polices exotiques.

Ce qu'on y perd est tout aussi net, et il faut le dire : la page n'est plus un
arbre mais une image. On ne peut ni la selectionner, ni la chercher, ni
l'inspecter, et il faut un hote qui tourne. Ce n'est pas *notre* navigateur qui
lit le web — c'est le sien.

## Le protocole

Volontairement minuscule : des requetes HTTP ordinaires, une image en reponse.

    /wv/open?url=&w=&h=      ouvre une adresse, rend l'image
    /wv/shot?w=&h=&sig=      l'image courante, ou 304 si elle n'a pas change
    /wv/click?x=&y=          un clic, puis l'image
    /wv/scroll?dy=           un defilement
    /wv/type?text=           du texte saisi
    /wv/key?k=               une touche nommee (Enter, Backspace, Tab…)
    /wv/back /wv/forward /wv/reload
    /wv/info                 l'adresse et le titre courants, en JSON

La signature evite de retransmettre une image identique : une page immobile ne
coute alors que trois octets par sondage, et toute la bande passante reste aux
pages qui bougent — une video, par exemple.

## Ou le trouver

Sous QEMU, l'hote est joignable a `10.0.2.2`. C'est la valeur par defaut ;
`BO_RENDU` la remplace, ce qui sert aussi bien a viser une autre machine qu'a
eprouver ce module sans emulateur.
"""

import json
import os
import urllib.parse

from . import reseau

# Adresse du service de rendu. `10.0.2.2` est l'hote vu depuis QEMU.
SERVICE = os.environ.get("BO_RENDU", "http://10.0.2.2:8080")

# Qualite JPEG demandee. En dessous de 60 le texte devient sale ; au-dessus de
# 80 le poids double sans gain visible a l'ecran d'une machine emulee.
QUALITE = 72

# Touches que le service accepte par leur nom.
TOUCHES = {
    "Enter", "Backspace", "Tab", "Escape", "Delete",
    "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "PageUp", "PageDown",
    "Home", "End",
}


class Session:
    """Une page distante, et la derniere image qu'on en a recue."""

    def __init__(self, service=None, journal=None):
        self.service = (service or SERVICE).rstrip("/")
        self.journal = journal or (lambda niveau, texte: None)
        self.largeur = 1024
        self.hauteur = 640
        self.url = ""
        self.titre = ""
        self.erreur = None
        # Octets de la derniere image, et sa signature cote service.
        self.octets = b""
        self.signature = ""

    # --- Actions --------------------------------------------------------------

    def ouvre(self, url, largeur=None, hauteur=None):
        """Ouvre une adresse. Rend `True` si une image est arrivee."""
        if largeur:
            self.largeur = int(largeur)
        if hauteur:
            self.hauteur = int(hauteur)
        return self._agit("open", {"url": url})

    def rafraichis(self):
        """Redemande l'image. Rend `True` si elle a change.

        C'est ce qui fait vivre une video : on redemande, et le service ne
        repond que lorsqu'il a autre chose a montrer.
        """
        return self._agit("shot", {}, garde_signature=True)

    def clic(self, x, y):
        return self._agit("click", {"x": int(x), "y": int(y)})

    def defile(self, dy):
        return self._agit("scroll", {"dy": int(dy)})

    def saisis(self, texte):
        if not texte:
            return False
        return self._agit("type", {"text": texte})

    def touche(self, nom):
        if nom not in TOUCHES:
            return False
        return self._agit("key", {"k": nom})

    def recule(self):
        return self._agit("back", {})

    def avance(self):
        return self._agit("forward", {})

    def recharge(self):
        return self._agit("reload", {})

    # --- Etat -----------------------------------------------------------------

    def etat(self):
        """Relit l'adresse et le titre courants du service."""
        reponse = self._demande("info", {})
        if reponse is None or reponse.code != 200:
            return False
        try:
            donnees = json.loads(reponse.octets.decode("utf-8", "replace"))
        except ValueError:
            return False
        self.url = donnees.get("url", "") or self.url
        self.titre = donnees.get("titre", "") or self.titre
        return True

    def joignable(self):
        """Le service repond-il ? Sert a decider si le mode distant est offert."""
        reponse = self._demande("healthz", {}, racine=True)
        return reponse is not None and reponse.code == 200

    # --- Mecanique ------------------------------------------------------------

    def _agit(self, action, parametres, garde_signature=False):
        demande = dict(parametres)
        demande["w"] = self.largeur
        demande["h"] = self.hauteur
        demande["fmt"] = "jpeg"
        demande["q"] = QUALITE
        # La signature n'accompagne que le sondage : une action a toujours de
        # bonnes chances d'avoir change la page, et un 304 la ferait manquer.
        if garde_signature and self.signature:
            demande["sig"] = self.signature

        reponse = self._demande(action, demande)
        if reponse is None:
            return False
        if reponse.code == 304:
            return False
        if reponse.code != 200 or not reponse.octets:
            self.erreur = "le service a rendu %s" % reponse.code
            self.journal("warn", "rendu distant: %s -> %s" % (action, reponse.code))
            return False

        self.erreur = None
        self.octets = reponse.octets
        self.signature = reponse.entetes.get("x-bo-signature", "")
        return True

    def _demande(self, chemin, parametres, racine=False):
        adresse = "%s/%s" % (self.service, chemin if racine else "wv/" + chemin)
        if parametres:
            adresse += "?" + urllib.parse.urlencode(parametres)
        try:
            return reseau.charge(adresse, brut=True)
        except Exception as e:  # noqa: BLE001 — un service absent n'est pas une panne
            self.erreur = str(e)
            self.journal("warn", "rendu distant injoignable : %s" % e)
            return None


def touche_pour(nom_touche):
    """Traduit une touche du navigateur en touche du service, ou `None`.

    Les noms viennent de l'hote Qt ; ceux du service sont ceux du DOM. La
    correspondance est explicite plutot que devinee : une touche mal traduite
    part quand meme, et fait agir la page distante a contretemps.
    """
    correspondance = {
        "entree": "Enter", "retour": "Backspace", "tabulation": "Tab",
        "echap": "Escape", "suppr": "Delete",
        "haut": "ArrowUp", "bas": "ArrowDown",
        "gauche": "ArrowLeft", "droite": "ArrowRight",
        "page_haut": "PageUp", "page_bas": "PageDown",
        "debut": "Home", "fin": "End",
    }
    return correspondance.get(nom_touche)
