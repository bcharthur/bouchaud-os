"""Les `<iframe>` : d'autres mondes Web dans la meme page.

## Ce que ce module tient

Un `<iframe>` n'est pas une boite avec un document dedans. C'est un element du
document parent qui **porte un contexte de navigation** — voir `contexte.py` —
et ce contexte a sa propre vie : son document, son `window`, son historique, son
origine, son viewport, son defilement.

Ce module fait le lien entre les deux : il regarde l'arbre du parent, cree un
contexte pour chaque `<iframe>` qui apparait, en detruit un pour chaque
`<iframe>` qui disparait, et navigue celui dont le `src` a change.

## Pourquoi la synchronisation plutot que des evenements

Le moteur ne dispose pas d'observateurs de mutation internes fiables sur tous
les chemins qui modifient l'arbre — `innerHTML`, `appendChild`, `insertAdjacent`,
l'analyse initiale. Plutot que d'ajouter un rappel a chacun et d'en oublier un,
on rapproche l'etat courant de l'arbre a chaque battement. C'est une operation
en O(elements) qui ne fait rien quand rien n'a change, et elle ne peut pas
manquer une mutation puisqu'elle ne depend d'aucune notification.

Le prix : un `<iframe>` cree et retire dans le meme battement n'existe jamais.
C'est sans consequence — il n'aurait rien pu charger.

## Le cycle de vie, et pourquoi il compte

    <iframe> apparait
      → BrowsingContext cree
      → Document charge
      → DOMContentLoaded, puis load sur l'element
    <iframe> retire de l'arbre
      → Document ferme (contexte QuickJS detruit, requetes abandonnees,
        connexions fermees, transactions liberees)
      → BrowsingContext detruit

Sans la seconde moitie, mille iframes crees et retires laissent mille contextes
QuickJS, mille pools de fils et mille connexions vivantes. C'est le genre de
fuite qui ne se voit pas sur une page d'essai et qui tue une application au bout
d'une heure.
"""

from . import contexte as mod_contexte
from . import html, origine as mod_origine, securite, telemetrie

# Au-dela, on refuse d'imbriquer : un document qui s'inclut lui-meme ferait une
# descente infinie, et c'est la premiere chose qu'une page hostile essaie.
PROFONDEUR_MAX = 8

# Nombre maximal de contextes vivants sous un meme sommet. Une page qui en
# fabrique dix mille epuise la memoire ; la borne la fait echouer proprement.
CADRES_MAX = 64


def _adresse(element, base):
    """L'adresse absolue que l'iframe demande, ou `None` s'il n'en demande pas."""
    brut = (element.attributs.get("src") or "").strip()
    if not brut:
        # Un iframe sans `src` porte un document vide de meme origine que son
        # parent. C'est ce que dit la norme, et c'est ce dont se servent les
        # pages qui ecrivent dans `contentDocument`.
        return "about:blank"
    import urllib.parse
    return urllib.parse.urljoin(base, brut)


def _dimensions(element, style, largeur_parent, hauteur_parent):
    """La taille du viewport de l'enfant.

    L'attribut HTML, la feuille de style, ou les valeurs par defaut de la norme
    (300x150). Ce n'est pas cosmetique : c'est le viewport auquel se rapportent
    les `@media` et les `vw`/`vh` du document enfant.
    """
    from . import css

    def mesure(nom_attribut, nom_css, reference, defaut):
        valeur = css.longueur(style.get(nom_css, "auto"), reference, 16.0)
        if valeur is not None and valeur > 0:
            return valeur
        brut = (element.attributs.get(nom_attribut) or "").strip()
        if brut:
            try:
                return max(1.0, float(brut.rstrip("px").strip()))
            except ValueError:
                pass
        return defaut

    return (mesure("width", "width", largeur_parent, 300.0),
            mesure("height", "height", hauteur_parent, 150.0))


class Gestionnaire:
    """Ce qui tient les contextes enfants d'un document, et les fait vivre."""

    __slots__ = ("document", "par_element")

    def __init__(self, document):
        self.document = document
        # `id(element) -> (element, BrowsingContext)`.
        #
        # La reference a l'element n'est pas decorative : sans elle, l'element
        # retire de l'arbre est libere, Python **reutilise son identifiant**
        # pour le suivant, et le gestionnaire croit reconnaitre un cadre deja
        # connu la ou il y en a un nouveau. Mesure : deux cents iframes crees
        # et retires n'en produisaient qu'un seul. La garder rend l'identifiant
        # unique pour toute la duree ou l'entree existe.
        self.par_element = {}

    # --- Synchronisation ------------------------------------------------------

    def synchronise(self, largeur_parent, hauteur_parent, styles=None):
        """Rapproche l'arbre des contextes de l'arbre du document.

        Rend `True` si quelque chose a change — un contexte cree, detruit ou
        navigue —, ce qui dit a l'appelant qu'il doit remettre en page.
        """
        parent = getattr(self.document, "contexte_navigation", None)
        if parent is None or parent.ferme:
            return False
        if parent.profondeur >= PROFONDEUR_MAX:
            # Trop profond : on ne cree plus rien. Les iframes existants
            # continuent de vivre, ils ne se reproduisent plus.
            return False

        vus = set()
        change = False
        for element in self.document.racine.parcours():
            if not isinstance(element, html.Element) or element.balise != "iframe":
                continue
            vus.add(id(element))
            if self._met_a_jour(element, parent, largeur_parent, hauteur_parent,
                                styles):
                change = True

        # Ce qui n'est plus dans l'arbre est ferme. C'est la moitie du cycle de
        # vie que l'on oublie, et celle qui fuit.
        for cle in [c for c in self.par_element if c not in vus]:
            _, enfant = self.par_element.pop(cle)
            enfant.ferme_contexte()
            telemetrie.compte("cadre.retires")
            change = True
        return change

    def _met_a_jour(self, element, parent, largeur_parent, hauteur_parent, styles):
        style = (styles or {}).get(id(element), {})
        largeur, hauteur = _dimensions(element, style, largeur_parent,
                                       hauteur_parent)
        cible = _adresse(element, self.document.url)
        entree = self.par_element.get(id(element))
        enfant = None if entree is None else entree[1]

        if enfant is None:
            if len(parent.sommet.descendants()) >= CADRES_MAX:
                telemetrie.compte("cadre.refuses_trop_nombreux")
                return False
            enfant = mod_contexte.BrowsingContext(
                parent=parent, url="about:blank",
                nom=(element.attributs.get("name") or "").strip(),
                element=element, largeur=largeur, hauteur=hauteur,
                navigue=self._charge, journal=self.document.journal)
            self.par_element[id(element)] = (element, enfant)
            telemetrie.compte("cadre.crees")
            # `remplace=True` : le `about:blank` initial ne doit pas rester dans
            # l'historique, sinon le premier « precedent » y ramenerait.
            enfant.navigue(cible, remplace=True)
            self._signale_charge(element, enfant)
            return True

        change = False
        if (enfant.largeur, enfant.hauteur) != (largeur, hauteur):
            enfant.largeur, enfant.hauteur = largeur, hauteur
            if enfant.document is not None:
                enfant.document.remet_en_page(largeur)
            change = True
        if enfant.url != cible:
            # `iframe.src = ...` navigue **l'enfant**, jamais le document
            # principal. C'est la distinction qui fait exister le contexte.
            #
            # Tant que l'enfant n'a rien charge de reel, on remplace : le
            # `about:blank` d'un iframe dont la page pose le `src` par script ne
            # doit pas rester dans l'historique, sinon le premier « precedent »
            # y ramenerait au lieu de sortir du cadre.
            jamais_charge = enfant.document is None or enfant.url == "about:blank"
            enfant.navigue(cible, remplace=jamais_charge)
            self._signale_charge(element, enfant)
            change = True
        return change

    # --- Chargement -----------------------------------------------------------

    def _charge(self, enfant, url):
        """Le rappel que `BrowsingContext.navigue` appelle. Pose le document."""
        from . import reseau

        ancien = enfant.document
        if ancien is not None:
            fermeture = getattr(ancien, "ferme_document", None)
            if fermeture is not None:
                fermeture()
            enfant.document = None

        if url in ("about:blank", "", None):
            reponse = reseau.Reponse(url or "about:blank", "<html><body></body></html>",
                                     "text/html", 200)
        else:
            # Un iframe est une sous-ressource : sa provenance est `DOCUMENT`,
            # donc une page distante ne peut pas y ouvrir un `file://`. Sans
            # cette verification, l'iframe serait le trou par lequel toute la
            # politique de schema s'echappe.
            requete = securite.requete_document(url, self.document.url,
                                                destination="document")
            try:
                securite.verifie(requete)
            except securite.Refus as refus:
                self.document.journal("warn", "iframe refuse : %s" % refus.raison)
                telemetrie.compte("cadre.refuses_politique")
                reponse = reseau.Reponse(url, "", "text/html", 0,
                                         erreur=refus.raison)
            else:
                reponse = reseau.charge(url)

        document = self._nouveau_document(reponse, enfant)
        enfant.document = document
        enfant.origine = mod_origine.Origine.de_url(reponse.url or url)
        # L'adresse retenue est celle **apres** redirection : c'est elle qui
        # decide de l'origine, et donc de ce que le parent aura le droit de lire.
        enfant.url = reponse.url or url
        if enfant.historique:
            enfant.historique[enfant.position_historique] = enfant.url
        telemetrie.compte("cadre.charges")
        return document

    def _nouveau_document(self, reponse, enfant):
        from . import Document

        # L'origine du contexte est posee **avant** de construire le document :
        # les scripts de l'enfant tournent pendant `Document.__init__`, et ils
        # doivent deja voir la bonne frontiere. La poser apres coup laissait le
        # document enfant se croire de premier niveau pendant toute son amorce,
        # donc voir son `parent` comme lui-meme — et lire le DOM d'en face.
        enfant.url = reponse.url or enfant.url
        enfant.origine = mod_origine.Origine.de_url(enfant.url)
        document = Document(reponse, enfant.largeur, journal=self.document.journal,
                            hauteur_fenetre=enfant.hauteur, precharge=False,
                            contexte_navigation=enfant)
        return document

    def _signale_charge(self, element, enfant):
        """Emet `load` sur l'element `<iframe>`, comme le veut la norme."""
        contexte_js = getattr(self.document, "contexte_js", None)
        if contexte_js is None:
            return
        try:
            contexte_js.evenement(element, "load")
        except Exception as e:  # noqa: BLE001 — un iframe ne tue pas la page
            self.document.journal("warn", "iframe load : %s" % e)

    # --- Battement et fermeture ----------------------------------------------

    def bat(self):
        """Fait avancer les documents enfants. Rend `True` si l'un a change."""
        change = False
        for _, enfant in list(self.par_element.values()):
            document = enfant.document
            if document is None or enfant.ferme:
                continue
            try:
                if document.rafraichis():
                    change = True
            except Exception as e:  # noqa: BLE001
                self.document.journal("warn", "iframe: %s" % e)
        return change

    def ferme_tout(self):
        """Detruit tous les contextes enfants. Appele a la fermeture du parent."""
        for _, enfant in list(self.par_element.values()):
            enfant.ferme_contexte()
        self.par_element.clear()

    def contexte_de(self, element):
        entree = self.par_element.get(id(element))
        return None if entree is None else entree[1]

    def elements(self):
        """Les couples `(element, contexte)`."""
        return list(self.par_element.values())
