"""Moteur web natif de Bouchaud OS.

Enchainement, dans l'ordre :

    reseau      URL   -> octets
    html        octets -> arbre de nœuds
    css         feuilles + attributs `style` -> regles
    mise_en_page arbre + regles -> boites positionnees
    peinture    boites -> liste d'affichage

L'hote Qt (`hote.cpp`) n'intervient qu'aux deux extremites : il fournit la
mesure du texte a la mise en page, et il peint la liste d'affichage. Tout ce
qu'il y a entre les deux est du Python, et se teste sans ecran.
"""

from . import css, html, mise_en_page, peinture, reseau

__all__ = ["css", "html", "mise_en_page", "peinture", "reseau", "Document"]


class Document:
    """Une page chargee : son arbre, ses regles, sa mise en page."""

    def __init__(self, reponse, largeur):
        self.url = reponse.url
        self.code = reponse.code
        self.erreur = reponse.erreur
        self.racine = html.analyse(reponse.contenu)
        self.titre = self._titre()
        self.regles = self._regles()
        self.largeur = largeur
        self.boite = None
        self.hauteur = 0.0
        self.zones_liens = []
        self.remet_en_page(largeur)

    def _titre(self):
        element = self.racine.trouve("title")
        if element:
            titre = " ".join(element.texte().split())
            if titre:
                return titre
        return self.url

    def _regles(self):
        regles = css.analyse(css.FEUILLE_PAR_DEFAUT)
        ordre = len(regles) + 1000
        for element in self.racine.parcours():
            if isinstance(element, html.Element) and element.balise == "style":
                nouvelles = css.analyse(element.texte(), ordre)
                regles.extend(nouvelles)
                ordre += len(nouvelles) + 1
        return regles

    def remet_en_page(self, largeur):
        self.largeur = largeur
        self.boite, self.hauteur = mise_en_page.construit(
            self.racine, self.regles, largeur)

    def liste_affichage(self, defilement, largeur_vue, hauteur_vue):
        self.zones_liens = []
        return peinture.peint(self.boite, defilement, largeur_vue, hauteur_vue,
                              self.zones_liens)

    def lien_a(self, x, y):
        """URL du lien sous ce point, en coordonnees de page. `None` sinon."""
        for zx, zy, zl, zh, url in self.zones_liens:
            if zx <= x <= zx + zl and zy <= y <= zy + zh:
                return url
        return None


def charge(url, largeur):
    """Charge une URL et rend le [`Document`] correspondant."""
    return Document(reseau.charge(url), largeur)
