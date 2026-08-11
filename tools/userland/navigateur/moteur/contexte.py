"""Contextes de navigation : plusieurs mondes Web a la fois.

## L'hypothese qui disparait

Tout le moteur reposait jusqu'ici sur une equivalence tacite :

    un navigateur = un Document = un contexte JavaScript

Elle a tenu tant qu'une page etait seule. Elle ne tient plus des le premier
`<iframe>` : il y a alors deux documents vivants, deux `window`, deux
historiques, deux origines — et surtout **une frontiere entre les deux**, qui
n'a de sens que si chacun sait a quel monde il appartient.

Un `BrowsingContext` est ce monde. Ce n'est pas un `Document` : le document
change a chaque navigation, le contexte lui survit. C'est ce qui permet a
`iframe.contentWindow` de rester valable apres que l'iframe a change de page, et
c'est pourquoi la norme les distingue.

## L'arbre

    Navigateur
    └── BrowsingContext de premier niveau
          ├── Document
          ├── Window (JavaScript)
          └── BrowsingContext d'iframe
                ├── Document
                └── Window

Le document principal n'est plus un cas particulier implicite : il possede son
contexte comme les autres, et c'est ce qui rend `window.top` et `window.parent`
calculables sans code special.

## Ce que ce module ne fait pas

Il ne charge rien, ne met rien en page, ne dessine rien. Il tient la structure,
les relations et le cycle de vie — le chargement est confie a un rappel fourni
par l'appelant. Cette separation n'est pas cosmetique : elle permet aux epreuves
de construire un arbre de contextes sans reseau, et elle est ce qui rendra
possible, plus tard, de faire vivre un contexte dans un autre processus sans
toucher a cette structure.
"""

import itertools

from . import origine as mod_origine
from . import telemetrie

_compteur = itertools.count(1)


class BrowsingContext:
    """Un monde Web : un document a la fois, une identite qui lui survit."""

    __slots__ = ("id", "parent", "enfants", "url", "document", "origine",
                 "largeur", "hauteur", "defilement_x", "defilement_y",
                 "actif", "focalise", "nom", "element", "historique",
                 "position_historique", "ferme", "_navigue", "journal")

    def __init__(self, parent=None, url="about:blank", nom="", element=None,
                 largeur=1000.0, hauteur=800.0, navigue=None, journal=None):
        self.id = next(_compteur)
        self.parent = parent
        self.enfants = []
        self.url = url
        self.document = None
        self.origine = mod_origine.Origine.de_url(url)
        # Le viewport de ce contexte. Celui d'un iframe est la boite de
        # l'iframe, pas la fenetre : c'est ce qui fait qu'une `@media` ou un
        # `vw` a l'interieur se rapportent au cadre et non a l'ecran.
        self.largeur = float(largeur)
        self.hauteur = float(hauteur)
        self.defilement_x = 0.0
        self.defilement_y = 0.0
        self.actif = True
        self.focalise = parent is None
        self.nom = nom
        # L'element `<iframe>` qui porte ce contexte, `None` au premier niveau.
        self.element = element
        # Chaque contexte a **son** historique. Les fondre en un seul tableau
        # etait la facon la plus sure de rendre le bouton « precedent »
        # incomprehensible : reculer dans un iframe n'est pas reculer dans la
        # page.
        self.historique = [url]
        self.position_historique = 0
        self.ferme = False
        # `navigue(contexte, url)` : charge un document et le pose sur le
        # contexte. Fourni par l'appelant — ce module ne connait pas le reseau.
        self._navigue = navigue
        self.journal = journal or (lambda niveau, texte: None)
        if parent is not None:
            parent.enfants.append(self)
        telemetrie.compte("contexte.crees")

    # --- Relations -----------------------------------------------------------

    @property
    def sommet(self):
        """Le contexte de premier niveau de cet arbre (`window.top`)."""
        courant = self
        while courant.parent is not None:
            courant = courant.parent
        return courant

    @property
    def est_sommet(self):
        return self.parent is None

    @property
    def profondeur(self):
        n, courant = 0, self
        while courant.parent is not None:
            n += 1
            courant = courant.parent
        return n

    def descendants(self):
        """Tous les contextes sous celui-ci, sans lui-meme."""
        sortie = []
        pile = list(self.enfants)
        while pile:
            contexte = pile.pop()
            sortie.append(contexte)
            pile.extend(contexte.enfants)
        return sortie

    def par_id(self, identifiant):
        """Retrouve un contexte de cet arbre par son identifiant."""
        if self.id == identifiant:
            return self
        for enfant in self.descendants():
            if enfant.id == identifiant:
                return enfant
        return None

    def par_nom(self, nom):
        """Le premier contexte de l'arbre portant ce nom (`window.frames.x`)."""
        if not nom:
            return None
        for contexte in [self.sommet] + self.sommet.descendants():
            if contexte.nom == nom:
                return contexte
        return None

    # --- Navigation ----------------------------------------------------------

    def navigue(self, url, remplace=False):
        """Charge une adresse dans **ce** contexte.

        `remplace` ecrase l'entree courante au lieu d'en ajouter une : c'est ce
        que fait `location.replace()`, et ce que doit faire le chargement
        initial d'un iframe — sans quoi le premier « precedent » ramenerait a
        l'`about:blank` que personne n'a demande.
        """
        if self.ferme:
            return False
        self.url = url
        self.origine = mod_origine.Origine.de_url(url)
        if remplace or not self.historique:
            self.historique[self.position_historique:] = [url]
        else:
            # Naviguer depuis le milieu de l'historique coupe ce qui suit :
            # c'est la branche qu'on abandonne en repartant.
            del self.historique[self.position_historique + 1:]
            self.historique.append(url)
            self.position_historique = len(self.historique) - 1
        telemetrie.compte("contexte.navigations")
        if self._navigue is not None:
            try:
                self._navigue(self, url)
            except Exception as e:  # noqa: BLE001 — un iframe ne tue pas la page
                self.journal("error", "iframe %s : %s" % (url, e))
                return False
        # Sans rappel de chargement, le contexte est navigue mais sans document.
        # L'adresse et l'historique changent quand meme : ils decrivent ou le
        # contexte se trouve, pas ce qu'il a reussi a charger. Les faire diverger
        # rendrait `location.href` et le bouton « precedent » incoherents.
        return True

    def recule(self):
        if self.position_historique <= 0:
            return False
        self.position_historique -= 1
        return self._rejoue()

    def avance(self):
        if self.position_historique >= len(self.historique) - 1:
            return False
        self.position_historique += 1
        return self._rejoue()

    def _rejoue(self):
        url = self.historique[self.position_historique]
        self.url = url
        self.origine = mod_origine.Origine.de_url(url)
        if self._navigue is not None:
            try:
                self._navigue(self, url)
            except Exception as e:  # noqa: BLE001
                self.journal("error", "iframe %s : %s" % (url, e))
                return False
        return True

    # --- Cycle de vie --------------------------------------------------------

    def ferme_contexte(self):
        """Detruit ce contexte et toute sa descendance, de bas en haut.

        L'ordre compte. Un enfant tient des ressources — un contexte QuickJS,
        des requetes en vol, des connexions, des transactions — qui doivent
        partir avant que son parent ne disparaisse, sinon un rappel arrive dans
        un monde qui n'existe plus. Detruire de bas en haut est la seule facon
        de garantir qu'a chaque etape, le destinataire de ce qu'on annule est
        encore la.

        Idempotent : fermer deux fois ne fait rien la seconde. C'est ce qui
        permet de fermer sans se demander si quelqu'un l'a deja fait.
        """
        if self.ferme:
            return False
        # De bas en haut : les enfants d'abord.
        for enfant in list(self.enfants):
            enfant.ferme_contexte()
        del self.enfants[:]

        document = self.document
        if document is not None:
            fermeture = getattr(document, "ferme_document", None)
            if fermeture is not None:
                try:
                    fermeture()
                except Exception as e:  # noqa: BLE001 — la fermeture n'echoue pas
                    self.journal("warn", "fermeture du document : %s" % e)
        self.document = None
        self.ferme = True
        self.actif = False
        if self.parent is not None and self in self.parent.enfants:
            self.parent.enfants.remove(self)
        self.parent = None
        telemetrie.compte("contexte.fermes")
        return True

    # --- Securite ------------------------------------------------------------

    def meme_origine(self, autre):
        """Ces deux contextes appartiennent-ils au meme monde ?

        C'est la question dont depend tout le reste : lire `contentDocument`,
        recevoir un `postMessage` sans `targetOrigin`, toucher au DOM d'en face.
        Elle passe par le type `Origine`, jamais par une comparaison de chaines.
        """
        if autre is None:
            return False
        return self.origine.meme_origine(autre.origine)

    def __repr__(self):
        return "BrowsingContext(#%d, %s, %s, %d enfant(s))" % (
            self.id, "sommet" if self.est_sommet else "iframe",
            self.origine.serialise(), len(self.enfants))


class TopLevelBrowsingContext(BrowsingContext):
    """Le contexte d'un onglet : celui qui n'a pas de parent.

    Une classe distincte plutot qu'un `parent is None` dissemine : c'est le
    point ou viendront les proprietes qui n'appartiennent qu'au premier niveau —
    le titre de l'onglet, la barre d'adresse, le groupe d'onglets liés — et
    l'existence du type empeche qu'un iframe s'en voie attribuer par megarde.
    """

    __slots__ = ()

    def __init__(self, url="about:blank", **reste):
        reste.pop("parent", None)
        super().__init__(parent=None, url=url, **reste)
