"""Analyseur HTML : du texte a un arbre de nœuds.

Ce n'est pas un analyseur conforme a HTML5 — celui-la fait des milliers de
lignes de regles de rattrapage. C'est un analyseur qui tient debout devant du
HTML reel : balises jamais fermees, imbrications interdites, attributs sans
guillemets, commentaires, entites. Les regles implementees sont celles qui, si
elles manquent, font s'effondrer l'arbre entier plutot que de deplacer un
paragraphe.
"""

import re

# Balises sans contenu : elles se ferment toutes seules.
VIDES = {
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link",
    "meta", "param", "source", "track", "wbr",
}

# Contenu pris tel quel, sans y chercher de balises.
BRUTES = {"script", "style", "textarea", "title"}

# Une balise ouvrante de la cle ferme implicitement celles de la valeur.
# Sans cela, une liste `<li>a<li>b` produirait des `li` imbriques a l'infini.
FERME_IMPLICITE = {
    "li": {"li"},
    "dt": {"dt", "dd"},
    "dd": {"dt", "dd"},
    "p": {"p"},
    "tr": {"tr", "td", "th"},
    "td": {"td", "th"},
    "th": {"td", "th"},
    "option": {"option"},
    "thead": {"tbody", "tfoot"},
    "tbody": {"thead", "tbody", "tfoot"},
}

# Une balise ouvrante ferme aussi tout `p` encore ouvert : un paragraphe ne
# peut pas contenir de bloc.
FERME_PARAGRAPHE = {
    "address", "article", "aside", "blockquote", "div", "dl", "fieldset",
    "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "header",
    "hr", "main", "nav", "ol", "p", "pre", "section", "table", "ul",
}

ENTITES = {
    "amp": "&", "lt": "<", "gt": ">", "quot": '"', "apos": "'", "nbsp": " ",
    "copy": "©", "reg": "®", "trade": "™", "hellip": "…", "mdash": "—",
    "ndash": "–", "laquo": "«", "raquo": "»", "eacute": "é", "egrave": "è",
    "ecirc": "ê", "agrave": "à", "ccedil": "ç", "ugrave": "ù", "ocirc": "ô",
    "icirc": "î", "acirc": "â", "euml": "ë", "iuml": "ï", "uuml": "ü",
    "deg": "°", "euro": "€", "middot": "·", "bull": "•", "rsquo": "’",
    "lsquo": "‘", "ldquo": "“", "rdquo": "”", "times": "×", "divide": "÷",
    "shy": "", "zwnj": "", "thinsp": " ",
}

_ENTITE = re.compile(r"&(#[0-9]{1,7}|#[xX][0-9a-fA-F]{1,6}|[a-zA-Z][a-zA-Z0-9]{1,31});?")


def decode_entites(texte):
    """Remplace `&amp;`, `&#233;`, `&#x2014;`... par leur caractere."""
    def remplace(m):
        corps = m.group(1)
        if corps.startswith("#"):
            try:
                code = int(corps[2:], 16) if corps[1] in "xX" else int(corps[1:])
                return chr(code) if 0 < code < 0x110000 else ""
            except ValueError:
                return m.group(0)
        return ENTITES.get(corps, m.group(0))
    return _ENTITE.sub(remplace, texte)


class Nœud:
    """Base commune : tout nœud connait son parent."""

    def __init__(self):
        self.parent = None


class Texte(Nœud):
    def __init__(self, contenu):
        super().__init__()
        self.contenu = contenu

    def __repr__(self):
        extrait = self.contenu[:30].replace("\n", " ")
        return "Texte(%r)" % extrait


class Element(Nœud):
    def __init__(self, balise, attributs=None):
        super().__init__()
        self.balise = balise
        self.attributs = attributs or {}
        self.enfants = []
        # `(chaine brute, liste decoupee)` : le decoupage de `class`, memorise.
        self._classes = None

    def ajoute(self, enfant):
        enfant.parent = self
        self.enfants.append(enfant)

    @property
    def classes(self):
        """Les classes de l'element, decoupees une seule fois.

        `class` est lu par chaque selecteur qui porte un point, donc des
        dizaines de milliers de fois par mise en page — 171 470 sur
        pypi.org/project/requests. Redecouper la meme chaine a chaque lecture
        etait du travail pur perdu.

        Le cache est invalide par `pose_attribut`, seul chemin par lequel
        `class` peut changer : ni le JavaScript ni le moteur n'ecrivent
        directement dans `attributs`.
        """
        brut = self.attributs.get("class", "")
        cache = self._classes
        if cache is not None and cache[0] == brut:
            return cache[1]
        decoupees = brut.split()
        self._classes = (brut, decoupees)
        return decoupees

    def pose_attribut(self, nom, valeur):
        """Ecrit un attribut en gardant les caches derives coherents."""
        self.attributs[nom] = valeur
        if nom == "class":
            self._classes = None

    @property
    def identifiant(self):
        return self.attributs.get("id", "")

    def texte(self):
        """Tout le texte de la sous-arborescence, concatene."""
        morceaux = []
        pile = list(self.enfants)
        while pile:
            n = pile.pop(0)
            if isinstance(n, Texte):
                morceaux.append(n.contenu)
            elif isinstance(n, Element) and n.balise not in BRUTES:
                pile = list(n.enfants) + pile
        return "".join(morceaux)

    def trouve(self, balise):
        """Premier descendant portant cette balise."""
        for n in self.parcours():
            if isinstance(n, Element) and n.balise == balise:
                return n
        return None

    def parcours(self):
        """Parcours prefixe de la sous-arborescence, self exclu."""
        for enfant in self.enfants:
            yield enfant
            if isinstance(enfant, Element):
                yield from enfant.parcours()

    def __repr__(self):
        return "<%s %d enfant(s)>" % (self.balise, len(self.enfants))


_BALISE = re.compile(r"<(/?)([a-zA-Z][a-zA-Z0-9:-]*)((?:[^>\"']|\"[^\"]*\"|'[^']*')*)>")
_ATTRIBUT = re.compile(r"""([a-zA-Z_:@][-a-zA-Z0-9_:.]*)\s*(?:=\s*("[^"]*"|'[^']*'|[^\s>]+))?""")


def _attributs(source):
    resultat = {}
    for m in _ATTRIBUT.finditer(source):
        nom = m.group(1).lower()
        valeur = m.group(2)
        if valeur is None:
            valeur = ""
        elif valeur[:1] in "\"'":
            valeur = valeur[1:-1]
        resultat[nom] = decode_entites(valeur)
    return resultat


def analyse(source):
    """Analyse un document et rend son element racine (`html`)."""
    racine = Element("html")
    pile = [racine]
    position = 0
    longueur = len(source)

    while position < longueur:
        debut = source.find("<", position)
        if debut < 0:
            _ajoute_texte(pile[-1], source[position:])
            break
        if debut > position:
            _ajoute_texte(pile[-1], source[position:debut])

        # Commentaires et declarations.
        if source.startswith("<!--", debut):
            fin = source.find("-->", debut + 4)
            position = longueur if fin < 0 else fin + 3
            continue
        if source.startswith("<!", debut) or source.startswith("<?", debut):
            fin = source.find(">", debut)
            position = longueur if fin < 0 else fin + 1
            continue

        m = _BALISE.match(source, debut)
        if not m:
            # `<` isole : c'est du texte, pas une balise.
            _ajoute_texte(pile[-1], "<")
            position = debut + 1
            continue

        fermante, balise, reste = m.group(1), m.group(2).lower(), m.group(3)
        position = m.end()

        if fermante:
            _ferme(pile, balise)
            continue

        _ouvre(pile, balise, _attributs(reste))

        # Contenu brut : on saute jusqu'a la fermeture correspondante sans y
        # chercher de balises. Sinon le premier `<` d'un script casserait tout.
        if balise in BRUTES and not reste.rstrip().endswith("/"):
            fin = _fin_brute(source, position, balise)
            if fin > position:
                pile[-1].ajoute(Texte(source[position:fin]))
            position = fin
            fermeture = source.find(">", fin)
            position = longueur if fermeture < 0 else fermeture + 1
            _ferme(pile, balise)

    return racine


def _fin_brute(source, position, balise):
    motif = re.compile(r"</\s*" + re.escape(balise), re.I)
    m = motif.search(source, position)
    return m.start() if m else len(source)


def _ajoute_texte(parent, brut):
    if not brut:
        return
    parent.ajoute(Texte(decode_entites(brut)))


def _ouvre(pile, balise, attributs):
    if balise in FERME_PARAGRAPHE and any(e.balise == "p" for e in pile):
        _ferme(pile, "p")
    a_fermer = FERME_IMPLICITE.get(balise)
    if a_fermer and pile[-1].balise in a_fermer:
        _ferme(pile, pile[-1].balise)

    element = Element(balise, attributs)
    pile[-1].ajoute(element)
    if balise not in VIDES:
        pile.append(element)


def _ferme(pile, balise):
    """Ferme la balise si elle est ouverte, en refermant ce qu'elle contient.

    Une fermeture qui ne correspond a rien est ignoree : c'est ce que font les
    navigateurs, et c'est ce qui evite de depiler la racine sur un `</div>` en
    trop.
    """
    for index in range(len(pile) - 1, 0, -1):
        if pile[index].balise == balise:
            del pile[index:]
            return
