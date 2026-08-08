"""Politique de securite web : qui a le droit de demander quoi.

Ce module n'est **pas** une implementation de la Same-Origin Policy ni de CORS.
Il porte le vocabulaire minimal dont le reste du moteur a besoin — origine,
provenance, destination — et la seule regle qui ne pouvait pas attendre : une
page distante ne lit pas le disque de Bouchaud OS.

## Pourquoi ce vocabulaire d'abord

L'erreur qui guette un moteur maison, c'est d'ajouter un `if` de securite au
fond de la couche transport. `reseau.charge` ne sait pas — et ne doit pas
savoir — si l'adresse vient de la barre d'adresse ou d'un `fetch()`. C'est
pourtant toute la difference : l'utilisateur qui tape `file:///etc/hosts` fait
un choix ; la page qui appelle `fetch("file:///etc/hosts")` en fait un a sa
place.

D'ou le seul concept introduit ici : une requete a une **provenance**.

* `NAVIGATION` — l'utilisateur a demande cette adresse : barre d'adresse,
  signet, clic sur un lien de premier niveau. Aucune restriction de schema.
* `DOCUMENT` — un document ou son script demande cette adresse : image,
  feuille de style, script, police, `fetch`, `XMLHttpRequest`, module. Le
  document demandeur est alors connu, et sa propre origine decide.

La regle appliquee aujourd'hui tient en une phrase : **un document distant
n'obtient pas de ressource locale.** Une page `file://` garde le droit de
charger ses propres images et feuilles voisines, sans quoi ouvrir un fichier
HTML du disque n'aurait plus de sens.

Ce qui n'est pas ici et qui viendra : `mode`, `credentials`, `redirect`, l'etat
CORS et la provenance de la reponse. La forme est prevue pour les accueillir
sans deplacer les appelants — voir `docs/BROWSER_COMPATIBILITY_ROADMAP.md`.
"""

import urllib.parse

# --- Provenances --------------------------------------------------------------

NAVIGATION = "navigation"
DOCUMENT = "document"

# Schemas qui donnent acces a l'appareil plutot qu'au reseau. Les demander
# depuis une page distante, c'est lire le disque de l'utilisateur.
SCHEMAS_LOCAUX = ("file", "bo")

# Schemas dont le contenu voyage avec la demande elle-meme : ils n'accedent a
# rien et restent donc toujours permis.
SCHEMAS_INERTES = ("data", "about", "blob")


class Refus(Exception):
    """Une requete refusee par la politique. Porte de quoi l'expliquer."""

    def __init__(self, cible, raison):
        super().__init__(raison)
        self.cible = cible
        self.raison = raison


def schema(url):
    """Le schema en minuscules, sans le `:`. `""` si l'adresse n'en a pas."""
    texte = str(url or "")
    coupe = texte.find(":")
    if coupe <= 0:
        return ""
    tete = texte[:coupe].lower()
    # Un schema est fait de lettres, chiffres, `+`, `-`, `.` et commence par une
    # lettre. Sans ce controle, `C:\dossier` passerait pour le schema « c ».
    if not tete[:1].isalpha():
        return ""
    for caractere in tete:
        if not (caractere.isalnum() or caractere in "+-."):
            return ""
    return tete


def est_local(url):
    """Vrai si l'adresse designe l'appareil et non le reseau."""
    return schema(url) in SCHEMAS_LOCAUX


def origine(url):
    """L'origine `(schema, hote, port)` de l'adresse, ou `None` si opaque.

    Une origine opaque — `data:`, `about:blank`, un fichier — n'est egale a
    aucune autre, pas meme a elle-meme au sens de la norme. On rend `None`
    plutot qu'un tuple pour que le code appelant ne puisse pas comparer deux
    origines opaques par megarde et les croire identiques.
    """
    sch = schema(url)
    if sch not in ("http", "https", "ws", "wss"):
        return None
    morceaux = urllib.parse.urlsplit(url)
    hote = (morceaux.hostname or "").lower()
    if not hote:
        return None
    port = morceaux.port
    if port is None:
        port = 443 if sch in ("https", "wss") else 80
    return (sch, hote, int(port))


def meme_origine(a, b):
    """Vrai si les deux adresses partagent une origine non opaque."""
    origine_a = origine(a)
    return origine_a is not None and origine_a == origine(b)


class Requete:
    """Le contexte d'une requete : d'ou elle part, ou elle va, pour quoi faire.

    `destination` reprend le vocabulaire de Fetch (`document`, `image`, `style`,
    `script`, `font`, `media`, `fetch`, `xhr`) et ne sert pour l'instant qu'aux
    messages et a la telemetrie. Le jour ou CORS arrivera, c'est ce meme objet
    qui portera `mode` et `credentials` : les appelants n'auront pas a bouger.
    """

    __slots__ = ("cible", "provenance", "document", "destination")

    def __init__(self, cible, provenance=NAVIGATION, document=None,
                 destination="document"):
        self.cible = cible
        self.provenance = provenance
        self.document = document
        self.destination = destination

    @property
    def origine(self):
        """L'origine du document qui demande, ou `None`."""
        return origine(self.document)


def verifie(requete):
    """Leve `Refus` si la politique interdit cette requete. Sinon ne fait rien."""
    if requete.provenance != DOCUMENT:
        # L'utilisateur decide de ce qu'il ouvre. C'est le seul acteur qui en a
        # le droit, et lui retirer `file://` reviendrait a retirer le
        # gestionnaire de fichiers du systeme.
        return

    if not est_local(requete.cible):
        return

    # A partir d'ici la cible est locale. Reste a savoir qui la demande.
    if requete.document is None:
        # Provenance « document » sans document identifie : on ne peut pas
        # prouver que la demande est legitime, donc on refuse. Un appelant qui
        # a vraiment le droit de lire le disque passe par `NAVIGATION`.
        raise Refus(requete.cible,
                    "ressource locale demandee sans document d'origine")

    if est_local(requete.document):
        # Une page du disque charge ses voisines : c'est le cas normal d'un
        # fichier HTML ouvert a la main.
        return

    raise Refus(
        requete.cible,
        "un document %s ne peut pas charger %s: — les fichiers de l'appareil "
        "ne sont accessibles qu'aux pages que l'utilisateur ouvre lui-meme"
        % (schema(requete.document) or "distant", schema(requete.cible)))


def requete_document(cible, document, destination="fetch"):
    """Raccourci : une requete emise par `document` vers `cible`."""
    return Requete(cible, DOCUMENT, document, destination)
