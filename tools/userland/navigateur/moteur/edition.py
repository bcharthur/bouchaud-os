"""L'etat d'un champ que l'on peut editer : valeur, curseur, selection.

## Pourquoi une abstraction plutot que des attributs

Jusqu'ici, la valeur d'un champ vivait dans son attribut `value`. C'est ce que
le HTML montre, et c'est pour cela que l'erreur est naturelle — mais la norme
distingue deux choses que cet attribut confond :

* la **valeur par defaut**, celle ecrite dans le document, que `form.reset()`
  restaure et qu'un rechargement retrouve ;
* la **valeur courante**, celle que l'utilisateur a tapee, que `input.value`
  rend et que le formulaire envoie.

Les confondre a une consequence visible : `reset()` ne peut plus rien restaurer,
puisqu'il a lui-meme ecrase ce qu'il devait remettre. Le modele ci-dessous porte
donc la valeur courante ; l'attribut reste la valeur par defaut.

Il porte aussi ce qu'aucun attribut ne peut porter : la position du curseur et
l'etendue de la selection. Ces deux-la n'existent nulle part dans le document,
et sans eux « taper » se reduit a « remplacer la valeur entiere ».

## Ce que ce module ne fait pas

Il ne dessine rien, n'ecoute aucun evenement et ne connait ni le DOM ni Qt. Il
recoit des ordres — insere, efface, deplace — et rend un etat. C'est ce qui
permet de l'eprouver sans navigateur, et de le brancher aussi bien sur une
frappe reelle que sur un scenario de verification.

Il ne gere pas non plus le retour a la ligne visuel : un `<textarea>` garde ses
`\\n`, et c'est la mise en page qui decide ou le texte se coupe. Melanger les
deux ferait dependre la position du curseur de la largeur de la fenetre, ce qui
est vrai a l'ecran mais faux dans le modele.
"""

# Les types d'`<input>` qui acceptent une saisie libre. Les autres — `checkbox`,
# `radio`, `submit`, `file`, `color`, `range` — ont une valeur, mais on ne la
# tape pas.
TYPES_EDITABLES = frozenset((
    "text", "search", "email", "password", "url", "tel", "number",
))


def est_editable(element):
    """Ce nœud accepte-t-il une saisie au clavier ?"""
    balise = getattr(element, "balise", None)
    if balise == "textarea":
        return "disabled" not in element.attributs
    if balise != "input":
        return False
    if "disabled" in element.attributs or "readonly" in element.attributs:
        return False
    return (element.attributs.get("type") or "text").lower() in TYPES_EDITABLES


def est_masque(element):
    """Faut-il cacher ce que le champ contient ?"""
    return (getattr(element, "balise", None) == "input"
            and (element.attributs.get("type") or "").lower() == "password")


def multiligne(element):
    return getattr(element, "balise", None) == "textarea"


PUCE = "•"


class Edition:
    """La valeur courante d'un champ, son curseur et sa selection.

    Le curseur est un indice dans la valeur : `0` avant le premier caractere,
    `len(valeur)` apres le dernier. La selection est l'intervalle
    `[debut, fin)` ; quand `debut == fin`, il n'y a pas de selection et c'est le
    curseur seul.

    `ancre` retient d'ou la selection est partie, ce qui permet a `Maj+Gauche`
    de la reduire quand elle a ete etendue vers la droite — sans elle, chaque
    `Maj+fleche` ne saurait qu'agrandir.
    """

    __slots__ = ("valeur", "debut", "fin", "ancre", "multiligne", "masque")

    def __init__(self, valeur="", multiligne=False, masque=False):
        self.valeur = str(valeur or "")
        self.debut = self.fin = len(self.valeur)
        self.ancre = self.debut
        self.multiligne = bool(multiligne)
        self.masque = bool(masque)

    # -- Lecture ---------------------------------------------------------------

    @property
    def curseur(self):
        """La position d'insertion : la fin de la selection, ou le curseur seul."""
        return self.fin

    @property
    def a_selection(self):
        return self.debut != self.fin

    def texte_affiche(self):
        """Ce que la peinture doit dessiner.

        Un champ de mot de passe garde sa vraie valeur ici et n'en montre que
        des puces. Masquer dans le modele — remplacer les caracteres — perdrait
        la valeur a envoyer ; masquer a la peinture la conserve, et c'est le
        seul endroit ou l'information n'a pas besoin d'exister.
        """
        return PUCE * len(self.valeur) if self.masque else self.valeur

    def selection(self):
        return (min(self.debut, self.fin), max(self.debut, self.fin))

    # -- Deplacements ----------------------------------------------------------

    def _borne(self, position):
        return max(0, min(len(self.valeur), int(position)))

    def pose_curseur(self, position, etend=False):
        """Place le curseur. `etend` conserve l'ancre et etire la selection."""
        position = self._borne(position)
        if etend:
            self.fin = position
            self.debut = self.ancre
        else:
            self.debut = self.fin = self.ancre = position
        return True

    def deplace(self, pas, etend=False):
        """`Gauche`/`Droite`, avec ou sans `Maj`.

        Sans `Maj`, une selection existante ne se deplace pas d'un caractere :
        elle s'effondre a son bord. C'est ce que fait tout editeur, et l'oublier
        donne l'impression que la fleche « saute » un caractere.
        """
        if not etend and self.a_selection:
            debut, fin = self.selection()
            return self.pose_curseur(debut if pas < 0 else fin)
        return self.pose_curseur(self.fin + pas, etend)

    def au_debut(self, etend=False):
        """`Home` : debut de la ligne courante, pas du champ entier."""
        return self.pose_curseur(self._debut_de_ligne(self.fin), etend)

    def a_la_fin(self, etend=False):
        return self.pose_curseur(self._fin_de_ligne(self.fin), etend)

    def _debut_de_ligne(self, position):
        if not self.multiligne:
            return 0
        coupe = self.valeur.rfind("\n", 0, position)
        return 0 if coupe < 0 else coupe + 1

    def _fin_de_ligne(self, position):
        if not self.multiligne:
            return len(self.valeur)
        coupe = self.valeur.find("\n", position)
        return len(self.valeur) if coupe < 0 else coupe

    def tout_selectionner(self):
        self.debut = self.ancre = 0
        self.fin = len(self.valeur)
        return True

    def pose_selection(self, debut, fin):
        """`setSelectionRange()`."""
        self.debut = self.ancre = self._borne(debut)
        self.fin = self._borne(fin)
        return True

    # -- Modifications ---------------------------------------------------------
    #
    # Chacune rend `True` si la valeur a change. C'est ce booleen qui decide si
    # `input` doit partir : une touche qui ne modifie rien — `Gauche` en debut
    # de champ, `Retour arriere` sur un champ vide — ne doit pas emettre
    # d'evenement, sans quoi toute validation « au changement » se declencherait
    # a chaque appui.

    def insere(self, texte):
        texte = str(texte)
        if not self.multiligne:
            # Un `<input>` est monoligne : un collage multiligne y devient une
            # seule ligne, comme dans tout navigateur, plutot que d'y injecter
            # des retours que rien ne saurait afficher.
            texte = texte.replace("\r\n", " ").replace("\n", " ").replace("\r", " ")
        else:
            texte = texte.replace("\r\n", "\n").replace("\r", "\n")
        if not texte:
            return False
        debut, fin = self.selection()
        self.valeur = self.valeur[:debut] + texte + self.valeur[fin:]
        self.debut = self.fin = self.ancre = debut + len(texte)
        return True

    def efface_avant(self):
        """`Retour arriere`."""
        debut, fin = self.selection()
        if debut != fin:
            return self._retire(debut, fin)
        if debut == 0:
            return False
        return self._retire(debut - 1, debut)

    def efface_apres(self):
        """`Suppr`."""
        debut, fin = self.selection()
        if debut != fin:
            return self._retire(debut, fin)
        if fin >= len(self.valeur):
            return False
        return self._retire(fin, fin + 1)

    def _retire(self, debut, fin):
        self.valeur = self.valeur[:debut] + self.valeur[fin:]
        self.debut = self.fin = self.ancre = debut
        return True

    def texte_selectionne(self):
        debut, fin = self.selection()
        return self.valeur[debut:fin]

    def remplace_tout(self, valeur):
        """Ecriture directe par un script : `input.value = "…"`.

        Le curseur va a la fin, comme le veut la norme quand la valeur est
        remplacee par programme.
        """
        valeur = str(valeur or "")
        change = valeur != self.valeur
        self.valeur = valeur
        self.debut = self.fin = self.ancre = len(valeur)
        return change


# --- Touches ------------------------------------------------------------------
#
# Le nom d'une touche suit `KeyboardEvent.key` : c'est ce que le JavaScript de la
# page verra, et avoir deux vocabulaires — un pour le moteur, un pour la page —
# aurait garanti qu'ils divergent.

DEPLACEMENTS = {
    "ArrowLeft": lambda e, maj: e.deplace(-1, maj),
    "ArrowRight": lambda e, maj: e.deplace(1, maj),
    "Home": lambda e, maj: e.au_debut(maj),
    "End": lambda e, maj: e.a_la_fin(maj),
}


def applique_touche(edition, touche, texte="", maj=False, ctrl=False):
    """Applique une frappe. Rend `(valeur_changee, curseur_bouge)`.

    Deux booleens plutot qu'un, parce que les deux ont des consequences
    differentes : la valeur qui change emet `input` et demande une remise en
    page ; le curseur qui bouge ne demande qu'un repeint.
    """
    if ctrl and touche in ("a", "A"):
        return False, edition.tout_selectionner()

    action = DEPLACEMENTS.get(touche)
    if action is not None:
        return False, action(edition, maj)

    if touche == "Backspace":
        return edition.efface_avant(), True
    if touche == "Delete":
        return edition.efface_apres(), True
    if touche == "Enter":
        # Dans un `<input>`, `Entree` soumet le formulaire ; il n'insere rien.
        # C'est a l'appelant de decider quoi en faire — le modele se contente
        # de ne pas ecrire.
        if not edition.multiligne:
            return False, False
        return edition.insere("\n"), True
    if touche == "Tab":
        return False, False

    # Le texte d'une touche imprimable. `texte` est ce que l'hote rapporte, et
    # c'est lui qui porte la disposition du clavier : deduire le caractere du
    # nom de la touche donnerait un clavier americain a tout le monde.
    if ctrl:
        return False, False
    if texte and texte.isprintable():
        return edition.insere(texte), True
    if len(touche) == 1 and touche.isprintable():
        return edition.insere(touche), True
    return False, False
