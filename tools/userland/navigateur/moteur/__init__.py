"""Moteur web natif de Bouchaud OS.

Enchainement, dans l'ordre :

    reseau      URL   -> octets
    html        octets -> arbre de nœuds
    css         feuilles + attributs `style` -> regles
    js          scripts de la page -> modifications de l'arbre
    mise_en_page arbre + regles -> boites positionnees
    peinture    boites -> liste d'affichage

L'hote Qt (`hote.cpp`) n'intervient qu'aux deux extremites : il fournit la
mesure du texte a la mise en page, le decodage des images, et il peint la liste
d'affichage. Tout ce qu'il y a entre les deux est du Python, et se teste sans
ecran.

## Le JavaScript, et quand il tourne

Les scripts s'executent une fois l'arbre construit, avant la premiere mise en
page : c'est ce qui permet a une page qui fabrique son contenu dans
`DOMContentLoaded` de s'afficher. Ensuite, le document se remet en page des que
le script touche a l'arbre — voir [`Document.rafraichis`], que le navigateur
appelle a son battement.

Une page sans script ne paie rien : le contexte JavaScript n'est cree que si
elle en contient au moins un.
"""

import urllib.parse

from . import (css, html, images, mise_en_page, peinture, prechargement,
               reseau)

__all__ = ["css", "html", "images", "js", "mise_en_page", "peinture",
           "prechargement", "reseau", "Document"]


class Document:
    """Une page chargee : son arbre, ses regles, ses scripts, sa mise en page."""

    def __init__(self, reponse, largeur, scripts=True, journal=None,
                 hauteur_fenetre=720.0, precharge=True):
        self.url = reponse.url
        self.code = reponse.code
        self.erreur = reponse.erreur
        self.racine = html.analyse(reponse.contenu)
        self.titre = self._titre()
        self.journal = journal or (lambda niveau, texte: None)
        # Feuilles liees deja rapportees, par adresse : la cascade est
        # rejouee a chaque redimensionnement, pas le telechargement.
        self._feuilles = {}
        # Index des regles, et signature des feuilles dont il est issu.
        self._index = None
        self._signature = None

        # Les sous-ressources partent maintenant, ensemble, pendant qu'on
        # analyse les feuilles : quand la mise en page reclamera une image, elle
        # sera deja la. C'est le seul endroit ou l'ordre compte — precharger
        # apres la mise en page ne servirait plus a rien.
        if precharge:
            prechargement.precharge(self.racine, self.url, self.journal)

        # La taille de la fenetre doit etre connue **avant** l'analyse des
        # feuilles : c'est elle qui decide quelles regles `@media` sont
        # retenues, et `vw`/`vh` s'y rapportent.
        self.hauteur_fenetre = float(hauteur_fenetre) or 720.0
        css.pose_fenetre(largeur, self.hauteur_fenetre)
        self.regles = self._regles()
        self.largeur = largeur
        self.boite = None
        self.hauteur = 0.0
        self.zones_liens = []
        self.contexte_js = None

        if scripts:
            self._demarre_scripts()
        self.remet_en_page(largeur)
        if self.contexte_js is not None:
            # `load` part apres la premiere mise en page : c'est la que
            # `getBoundingClientRect` a enfin des valeurs a rendre.
            self.contexte_js.signale_pret()
            self.rafraichis()

    # --- Scripts --------------------------------------------------------------

    def _a_des_scripts(self):
        for element in self.racine.parcours():
            if isinstance(element, html.Element) and element.balise == "script":
                return True
        return False

    def _demarre_scripts(self):
        if not self._a_des_scripts():
            return
        try:
            from . import js
        except ImportError as e:  # hote sans QuickJS : la page reste statique
            self.journal("warn", "JavaScript indisponible : %s" % e)
            return
        try:
            self.contexte_js = js.Contexte(self, journal=self.journal)
            self.contexte_js.execute_scripts()
        except Exception as e:  # noqa: BLE001 — un script ne tue pas la page
            self.journal("error", "JavaScript : %s" % e)
            self.contexte_js = None

    def rafraichis(self):
        """Remet en page si le JavaScript a touche a l'arbre. Rend `True` alors.

        A appeler au battement du navigateur : c'est ce qui fait qu'un
        `setTimeout` qui change le DOM se voit reellement a l'ecran.
        """
        contexte = self.contexte_js
        if contexte is None:
            return False
        contexte.tic()
        if not contexte.sale:
            return False
        contexte.sale = False
        # Les regles peuvent avoir change : un script qui insere un `<style>`
        # est un cas courant des bibliotheques de composants.
        self.regles = self._regles()
        self.titre = self.titre or self._titre()
        self.remet_en_page(self.largeur)
        return True

    def evenement_js(self, nœud, type_, details=None):
        """Distribue un evenement d'interface dans la page. Rend `False` si le
        script a demande d'annuler l'action par defaut (`preventDefault`)."""
        if self.contexte_js is None:
            return True
        resultat = self.contexte_js.evenement(nœud, type_, details)
        self.rafraichis()
        return resultat is not False

    @property
    def navigation_demandee(self):
        """URL que le script a demandee par `location.href = …`, ou `None`."""
        if self.contexte_js is None:
            return None
        demande = self.contexte_js.navigation
        self.contexte_js.navigation = None
        return demande

    def ferme(self):
        if self.contexte_js is not None:
            self.contexte_js.ferme()
            self.contexte_js = None

    # --- Arbre et mise en page ------------------------------------------------

    def _titre(self):
        element = self.racine.trouve("title")
        if element:
            titre = " ".join(element.texte().split())
            if titre:
                return titre
        return self.url

    def _regles(self):
        """Cascade complete : feuille de l'agent, feuilles liees, feuilles en ligne.

        L'ordre du document decide entre deux regles de meme specificite ; un
        `<link>` et un `<style>` sont donc numerotes dans l'ordre ou ils
        apparaissent, et non par categorie. Une feuille liee qui suit un
        `<style>` doit pouvoir le contredire, comme dans tout navigateur.
        """
        sources = []
        for element in self.racine.parcours():
            if not isinstance(element, html.Element):
                continue
            if element.balise == "style":
                sources.append(element.texte())
            elif element.balise == "link":
                sources.append(self._feuille_liee(element))

        # Analyser une feuille de deux mille regles a chaque battement du
        # JavaScript coute plus que toute la mise en page. Tant que les feuilles
        # sont les memes — texte compris, car un script peut reecrire un
        # `<style>` — on garde l'index deja construit.
        signature = tuple(sources)
        if signature == self._signature and self._index is not None:
            return self._index

        regles = css.analyse(css.FEUILLE_PAR_DEFAUT)
        ordre = len(regles) + 1000
        for source in sources:
            if not source:
                continue
            nouvelles = css.analyse(source, ordre)
            regles.extend(nouvelles)
            ordre += len(nouvelles) + 1

        self._signature = signature
        self._index = css.indexe(regles)
        return self._index

    def _feuille_liee(self, element):
        """Le texte d'un `<link rel="stylesheet">`, ou `""`.

        Le prechargement a en principe deja rapporte le fichier ; sinon on le
        demande ici. Une feuille absente ne fait pas echouer la page : elle
        s'affiche avec les regles qu'on a, ce que fait aussi un navigateur quand
        une requete echoue.
        """
        rel = element.attributs.get("rel", "").lower().split()
        if "stylesheet" not in rel:
            return ""
        adresse = (element.attributs.get("href") or "").strip()
        if not adresse:
            return ""
        media = element.attributs.get("media", "")
        if media and not css.requete_verifiee(media):
            return ""

        url = urllib.parse.urljoin(self.url or "", adresse)
        cle = url
        if cle in self._feuilles:
            return self._feuilles[cle]
        try:
            reponse = reseau.charge(url, brut=True)
        except Exception as e:  # noqa: BLE001
            self.journal("warn", "feuille %s : %s" % (adresse, e))
            self._feuilles[cle] = ""
            return ""
        texte = "" if (reponse.code and reponse.code >= 400) else reponse.contenu
        self._feuilles[cle] = texte
        return texte

    def remet_en_page(self, largeur, hauteur_fenetre=None):
        self.largeur = largeur
        if hauteur_fenetre:
            self.hauteur_fenetre = float(hauteur_fenetre)
        # Une fenetre redimensionnee change les `@media` retenues : les regles
        # sont donc reanalysees, pas seulement reappliquees.
        ancienne = css.fenetre()
        css.pose_fenetre(largeur, self.hauteur_fenetre)
        if ancienne != css.fenetre():
            # Les feuilles n'ont pas change, mais les `@media` retenues si :
            # l'index memorise ne vaut plus, il faut le reconstruire.
            self._signature = None
            self.regles = self._regles()
        self.boite, self.hauteur = mise_en_page.construit(
            self.racine, self.regles, largeur, self.url, self._image_video,
            self._toile)

    def _toile(self, nœud):
        """Ce qu'un `<canvas>` a dessine, ou `None` s'il n'a rien dessine."""
        if self.contexte_js is None:
            return None
        return self.contexte_js.toile(nœud)

    def _image_video(self, nœud):
        """Image courante d'un `<video>`, ou `None` s'il n'y en a pas encore."""
        if self.contexte_js is None:
            return None
        return self.contexte_js.image_video(nœud)

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

    def element_a(self, x, y):
        """Element sous ce point, en coordonnees de page. `None` sinon.

        Sert a designer la cible d'un clic pour le JavaScript : sans elle, un
        `addEventListener('click', …)` pose sur un bouton ne partirait jamais.
        """
        return _element_a(self.boite, x, y)


def _element_a(boite, x, y):
    if boite is None:
        return None
    if not (boite.x <= x <= boite.x + boite.largeur
            and boite.y <= y <= boite.y + boite.hauteur):
        return None
    # Le plus profond gagne : c'est lui que l'utilisateur vise.
    for enfant in boite.enfants:
        trouve = _element_a(enfant, x, y)
        if trouve is not None:
            return trouve
    return boite.element


def charge(url, largeur, scripts=True, journal=None):
    """Charge une URL et rend le [`Document`] correspondant."""
    return Document(reseau.charge(url), largeur, scripts=scripts, journal=journal)
