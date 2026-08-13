"""L'origine : l'unite de confiance du Web.

## Pourquoi un type, et pas une chaine

Jusqu'ici le moteur transportait l'origine sous forme de tuple, quand il la
transportait, et sous forme de chaine ailleurs — `stockage.py` compare
`"https://a.test"`, `indexeddb.py` en fabrique une autre a sa facon. Tant qu'il
n'y avait qu'un seul document vivant, l'incoherence ne se voyait pas.

Elle se verrait maintenant. Des l'instant ou deux documents coexistent, l'origine
cesse d'etre une etiquette et devient **la frontiere** : c'est elle qui decide si
un `iframe` peut lire le DOM de son parent, si un `postMessage` est livre, si un
`fetch` expose sa reponse, si un `canvas` reste lisible. Une comparaison
approximative a cet endroit n'est pas un defaut de style, c'est une faille.

Les pieges que le type ecarte, et qu'une comparaison de chaines ne voit pas :

* `https://a.test` et `https://a.test:443` sont **la meme** origine — le port
  par defaut du schema ne s'ecrit pas ;
* `http://a.test` et `https://a.test` sont **differentes** — le schema fait
  partie de l'origine, et c'est tout l'interet de HTTPS ;
* `https://sub.a.test` et `https://a.test` sont **differentes** — il n'y a pas
  d'heritage entre un sous-domaine et son parent ;
* `A.TEST` et `a.test` sont **la meme** — l'hote ne distingue pas la casse ;
* deux `data:` ou deux `about:blank` sont **differentes l'une de l'autre**, et
  meme differentes d'elles-memes au sens de la comparaison naive. Ce sont des
  origines **opaques**, et la norme veut qu'elles ne correspondent a rien.

Ce dernier point est celui qu'un tuple ne pouvait pas porter. Rendre `None`,
comme le faisait `securite.origine`, obligeait chaque appelant a se souvenir de
tester — et un appelant qui oublie compare `None == None` et conclut « meme
origine » pour deux pages qui n'ont rien a voir. Une origine opaque est ici un
objet a part entiere qui n'est egal qu'a lui-meme, par identite.
"""

import itertools
import urllib.parse

# Les schemas qui portent une origine tuple (schema, hote, port). Les autres —
# `file:`, `data:`, `about:`, `blob:` — sont opaques.
SCHEMAS_HIERARCHIQUES = ("http", "https", "ws", "wss")

PORTS_PAR_DEFAUT = {"http": 80, "https": 443, "ws": 80, "wss": 443}

# Les schemas qui parlent au meme serveur : `ws://` et `http://` d'un meme hote
# partagent une origine, ce dont depend l'ouverture d'un WebSocket vers son
# propre site.
SCHEMA_CANONIQUE = {"ws": "http", "wss": "https"}

_compteur_opaques = itertools.count(1)


class Origine:
    """Un triplet (schema, hote, port), ou une origine opaque unique."""

    __slots__ = ("schema", "hote", "port", "_opaque")

    def __init__(self, schema=None, hote=None, port=None, opaque=None):
        self.schema = schema
        self.hote = hote
        self.port = port
        # Un numero unique pour une origine opaque : c'est lui qui fait qu'une
        # `data:` n'est egale a aucune autre, pas meme a une `data:` identique.
        self._opaque = opaque

    # -- Construction ---------------------------------------------------------

    @classmethod
    def opaque(cls):
        """Une origine qui n'est egale qu'a elle-meme."""
        return cls(opaque=next(_compteur_opaques))

    @classmethod
    def de_url(cls, url):
        """L'origine d'une adresse. Opaque si le schema n'en porte pas."""
        texte = str(url or "")
        morceaux = urllib.parse.urlsplit(texte)
        schema = (morceaux.scheme or "").lower()
        if schema not in SCHEMAS_HIERARCHIQUES:
            return cls.opaque()
        hote = (morceaux.hostname or "").lower()
        if not hote:
            return cls.opaque()
        try:
            port = morceaux.port
        except ValueError:
            # Un port illisible (`http://a.test:abc`) ne donne pas une origine
            # par defaut : il donne une adresse invalide, donc une origine
            # opaque. Retomber sur 80 laisserait passer une adresse forgee.
            return cls.opaque()
        # Le schema est canonise **apres** la lecture du port, pour que
        # `wss://a.test` prenne bien 443 et non 80.
        defaut = PORTS_PAR_DEFAUT.get(schema, 0)
        if port is None:
            port = defaut
        return cls(SCHEMA_CANONIQUE.get(schema, schema), hote, int(port))

    # -- Comparaison ----------------------------------------------------------

    @property
    def est_opaque(self):
        return self._opaque is not None

    def meme_origine(self, autre):
        """La regle de la norme, et rien de plus.

        Deux origines opaques ne correspondent jamais, **meme la meme**, sauf a
        etre litteralement le meme objet — ce qui est le cas d'un document et de
        lui-meme.
        """
        if not isinstance(autre, Origine):
            return False
        if self.est_opaque or autre.est_opaque:
            return self is autre
        return (self.schema == autre.schema
                and self.hote == autre.hote
                and self.port == autre.port)

    def __eq__(self, autre):
        return self.meme_origine(autre)

    def __ne__(self, autre):
        return not self.__eq__(autre)

    def __hash__(self):
        if self.est_opaque:
            return hash(("opaque", self._opaque))
        return hash((self.schema, self.hote, self.port))

    # -- Serialisation --------------------------------------------------------

    def serialise(self):
        """Ce que la page voit dans `location.origin` et `event.origin`.

        Une origine opaque se serialise en `"null"` — c'est ce que dit la norme,
        et c'est ce qu'un `postMessage` depuis une page `data:` doit annoncer.
        Le port par defaut du schema ne s'ecrit pas : `https://a.test`, jamais
        `https://a.test:443`, sans quoi deux pages de la meme origine liraient
        deux chaines differentes.
        """
        if self.est_opaque:
            return "null"
        if self.port == PORTS_PAR_DEFAUT.get(self.schema):
            return "%s://%s" % (self.schema, self.hote)
        return "%s://%s:%d" % (self.schema, self.hote, self.port)

    def cle_stockage(self):
        """La cle sous laquelle ranger ce qui appartient a cette origine.

        Distincte de `serialise()` par prudence : une origine opaque n'a pas de
        stockage partage, et lui donner la cle `"null"` ferait que deux pages
        `data:` sans rapport se relisent l'une l'autre.
        """
        if self.est_opaque:
            return "opaque:%d" % self._opaque
        return self.serialise()

    def __repr__(self):
        return "Origine(%s)" % self.serialise()

    def __str__(self):
        return self.serialise()


def de_url(url):
    """Raccourci de lecture."""
    return Origine.de_url(url)


def meme_origine(a, b):
    """Deux adresses partagent-elles une origine non opaque ?

    Prend des adresses, pas des origines : c'est la forme dont le reste du
    moteur a besoin, et elle evite au lecteur de se demander si l'opacite a ete
    traitee.
    """
    return Origine.de_url(a).meme_origine(Origine.de_url(b))
