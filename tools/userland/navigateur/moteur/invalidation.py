"""Ce qu'il faut refaire quand quelque chose change — et rien de plus.

## Le modele qu'on remplace

Jusqu'ici le moteur n'avait qu'un booleen : `sale`. N'importe quelle mutation
le levait, et le battement suivant refaisait **tout** — cascade, mise en page,
peinture — pour la page entiere. Sur une page de 2 300 elements, changer la
couleur d'un bouton coutait exactement le meme prix que recharger le document.

Ce n'etait pas absurde tant que le DOM bougeait peu. Ce n'est plus tenable :
une application Web modifie son arbre a chaque interaction, et le navigateur
sait maintenant recevoir des frappes, donc le fait a la vitesse du clavier.

## Les quatre etendues

Le pipeline va du style aux pixels, et chaque etage depend du precedent :

    style  →  mise en page  →  peinture  →  composition

Une invalidation nomme donc l'etage le plus haut qu'il faut refaire ; tout ce
qui suit est refait aussi, tout ce qui precede est conserve.

* `STYLE` — la cascade doit etre rejouee : une classe a change, un attribut,
  une feuille est arrivee.
* `MISE_EN_PAGE` — les boites doivent etre replacees : une largeur, une marge,
  un `display`, du texte insere ou retire.
* `PEINTURE` — les boites sont bonnes, seuls les pixels changent : une couleur,
  un fond, une ombre, une bordure.
* `COMPOSITION` — meme les pixels sont bons ; il ne reste qu'a redeplacer ce
  qui est deja dessine : `transform`, `opacity`, un defilement.

## Ce que ce module fait, et ce qu'il ne fait pas encore

Il fait : classer une propriete CSS par l'etage qu'elle touche, retenir
l'etendue la plus haute demandee depuis le dernier rendu, et retenir **quels**
elements sont concernes.

Il ne fait pas : la propagation fine aux ancetres et aux descendants. Le moteur
consomme aujourd'hui l'etendue globale — « faut-il remettre en page ? » — et
ignore encore la liste des elements. C'est volontaire : poser la structure sans
la brancher partout permet de la verifier, et de brancher ensuite chaque etage
avec sa propre preuve. Une invalidation trop etroite se voit comme un bogue
d'affichage, la pire espece de bogue a diagnostiquer.
"""

# Les etendues, du plus cher au moins cher. L'ordre est signifiant : c'est lui
# qui permet de dire « la plus haute demandee » par un simple `max`.
RIEN = 0
COMPOSITION = 1
PEINTURE = 2
MISE_EN_PAGE = 3
STYLE = 4

NOMS = {
    RIEN: "RIEN",
    COMPOSITION: "COMPOSITION",
    PEINTURE: "PEINTURE",
    MISE_EN_PAGE: "MISE_EN_PAGE",
    STYLE: "STYLE",
}


# Ce qu'une propriete coute quand elle change. Ce classement n'est pas une
# opinion : il decoule de ce que le moteur lit ou non pendant chaque etage.
#
# `transform` et `opacity` sont en composition parce que la peinture les emet
# comme enveloppes de la liste d'affichage — l'hote les applique, les boites en
# dessous ne bougent pas. C'est la meme raison qui en fait des candidats
# naturels aux couches, plus tard.
_PAR_PROPRIETE = {}


def _range(etendue, proprietes):
    for nom in proprietes.split():
        _PAR_PROPRIETE[nom] = etendue


_range(COMPOSITION, """
transform transform-origin opacity translate rotate scale
""")

_range(PEINTURE, """
color background background-color background-image background-position
background-size background-repeat background-clip background-attachment
border-color border-top-color border-right-color border-bottom-color
border-left-color
box-shadow text-shadow text-decoration text-decoration-color
outline outline-color outline-offset outline-style
border-radius visibility filter backdrop-filter mix-blend-mode
cursor caret-color accent-color
""")

_range(MISE_EN_PAGE, """
display position top right bottom left z-index float clear
width height min-width min-height max-width max-height
margin margin-top margin-right margin-bottom margin-left
padding padding-top padding-right padding-bottom padding-left
border border-width border-top-width border-right-width border-bottom-width
border-left-width border-style box-sizing
font font-family font-size font-weight font-style line-height
text-align text-transform white-space vertical-align
flex flex-direction flex-wrap flex-grow flex-shrink flex-basis flex-flow
justify-content align-items align-self align-content
grid grid-template grid-template-columns grid-template-rows grid-template-areas
grid-area grid-column grid-row gap row-gap column-gap
order overflow overflow-x overflow-y aspect-ratio object-fit
list-style list-style-type content
""")


def etendue_de(propriete):
    """L'etage a refaire quand cette propriete change.

    Une propriete inconnue vaut `MISE_EN_PAGE` : c'est le choix prudent. Se
    tromper vers le haut coute du temps ; se tromper vers le bas laisse un
    affichage faux, et un affichage faux ne se voit pas dans un profil.
    """
    return _PAR_PROPRIETE.get(propriete, MISE_EN_PAGE)


def etendue_des(proprietes):
    """L'etage a refaire pour tout un lot de proprietes."""
    haute = RIEN
    for propriete in proprietes:
        etage = etendue_de(propriete)
        if etage > haute:
            haute = etage
            if haute == STYLE:
                break
    return haute


class Invalidation:
    """Ce qui reste a refaire, et pour qui.

    Un seul objet par document. Il accumule entre deux rendus et se vide quand
    le rendu a eu lieu — ce qui est le seul moment ou il est vrai de dire que
    plus rien n'est en retard.
    """

    __slots__ = ("etendue", "elements", "raisons")

    def __init__(self):
        self.etendue = RIEN
        # Les elements nommement touches. Vide ne veut pas dire « aucun » mais
        # « on ne sait pas lesquels » — une feuille de style qui arrive touche
        # potentiellement tout le document.
        self.elements = set()
        # Ce qui a provoque l'invalidation, pour le diagnostic et les
        # verifications. Borne : une page qui mute en boucle ne doit pas faire
        # grossir cette liste sans fin.
        self.raisons = []

    def marque(self, etendue, element=None, raison=""):
        """Demande un etage. Rend `True` si cela eleve ce qui etait deja prevu."""
        monte = etendue > self.etendue
        if monte:
            self.etendue = etendue
        if element is not None:
            self.elements.add(id(element))
        if raison and len(self.raisons) < 32:
            self.raisons.append("%s:%s" % (NOMS.get(etendue, etendue), raison))
        return monte

    def marque_propriete(self, propriete, element=None):
        """Invalide selon ce qu'une propriete CSS coute."""
        return self.marque(etendue_de(propriete), element, propriete)

    def marque_proprietes(self, proprietes, element=None):
        return self.marque(etendue_des(proprietes), element,
                           ",".join(sorted(proprietes))[:40])

    # -- Lectures --------------------------------------------------------------

    @property
    def rien_a_faire(self):
        return self.etendue == RIEN

    @property
    def demande_style(self):
        return self.etendue >= STYLE

    @property
    def demande_mise_en_page(self):
        return self.etendue >= MISE_EN_PAGE

    @property
    def demande_peinture(self):
        return self.etendue >= PEINTURE

    def vide(self):
        """Le rendu a eu lieu : plus rien n'est en retard."""
        etait = self.etendue
        self.etendue = RIEN
        self.elements.clear()
        del self.raisons[:]
        return etait

    def __repr__(self):
        return "Invalidation(%s, %d element(s))" % (
            NOMS.get(self.etendue, self.etendue), len(self.elements))
