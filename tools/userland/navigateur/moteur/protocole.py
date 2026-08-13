"""Le fil entre le navigateur et son processus de rendu.

Ce module ne fait que deux choses : encadrer un message, et refuser ce qui
n'en est pas un. Il ne connait ni le DOM, ni Qt, ni les pixels — c'est
volontaire, parce que c'est le seul endroit du systeme ou les deux cotes
doivent etre d'accord mot pour mot.

## La trame

    +--------+--------+----------------+---------------------+
    | version| genre  |    longueur    |      charge         |
    |  u16   |  u16   |      u32       |   `longueur` octets |
    +--------+--------+----------------+---------------------+

Gros-boutiste, huit octets d'en-tete. Trois champs, et chacun repond a une
question qu'un lecteur se pose necessairement :

* **version** — « est-ce que je sais lire ce qui suit ? ». Un renderer d'une
  version differente doit etre refuse a la premiere trame, pas au premier champ
  qui manque ;
* **genre** — ce que le message demande ;
* **longueur** — combien lire. C'est le champ dangereux : un pair hostile ou
  casse y met n'importe quoi, et un lecteur naif alloue ce qu'on lui dit.

## Ce qui est verifie, et pourquoi chaque verification existe

* **la version exacte.** Pas de « superieur ou egal » : deux versions qui se
  croient compatibles et ne le sont pas donnent une corruption silencieuse,
  la ou un refus donne un message d'erreur ;
* **le genre connu.** Un genre inconnu est refuse plutot qu'ignore. Ignorer
  reviendrait a laisser un pair croire qu'il a ete entendu ;
* **la longueur bornee.** `CHARGE_MAX` est petite a dessein : rien de ce qui
  passe par ce canal n'est gros. Les pixels voyagent par la memoire partagee,
  et une trame de controle qui approcherait le mebioctet serait forcement une
  erreur — soit un defaut, soit une attaque ;
* **la lecture complete.** Un `read` sur une prise rend ce qu'il a, pas ce
  qu'on demande. Traiter un en-tete tronque comme un en-tete valide est
  l'erreur classique de ce genre de code, et elle ne se voit qu'un jour de
  charge.

## Pourquoi du JSON dans la charge

Parce que ces messages sont **petits et rares** : une navigation, un
redimensionnement, une frappe. Le seul flux volumineux — la liste d'affichage —
ne passe pas par ici, il passe par la surface partagee. Choisir un encodage
binaire pour le controle aurait coute un analyseur de plus a ecrire et a
verifier, sans rien gagner de mesurable.

Le jour ou un message de controle deviendra chaud, c'est ce commentaire qu'il
faudra contredire — pas la structure, qui ne change pas.
"""

import json
import socket
import struct

VERSION = 1

# En-tete : version, genre, longueur.
EN_TETE = struct.Struct("!HHI")
TAILLE_EN_TETE = EN_TETE.size

# Une trame de controle qui depasse cela est forcement une erreur : la plus
# grosse charge legitime est une URL avec ses parametres.
CHARGE_MAX = 1 << 20

# --- Navigateur -> Renderer ---------------------------------------------------

CREATE_DOCUMENT = 1
NAVIGATE = 2
RESIZE = 3
INPUT_EVENT = 4
TICK = 5
SURFACE = 6
CLOSE = 7

# --- Renderer -> Navigateur ---------------------------------------------------

READY = 128
TITLE_CHANGED = 129
URL_CHANGED = 130
CURSOR_CHANGED = 131
FRAME_READY = 132
CONSOLE_MESSAGE = 133
ERROR = 134
REQUEST_NAVIGATION = 135

NOMS = {
    CREATE_DOCUMENT: "CREATE_DOCUMENT", NAVIGATE: "NAVIGATE", RESIZE: "RESIZE",
    INPUT_EVENT: "INPUT_EVENT", TICK: "TICK", SURFACE: "SURFACE",
    CLOSE: "CLOSE",
    READY: "READY", TITLE_CHANGED: "TITLE_CHANGED", URL_CHANGED: "URL_CHANGED",
    CURSOR_CHANGED: "CURSOR_CHANGED", FRAME_READY: "FRAME_READY",
    CONSOLE_MESSAGE: "CONSOLE_MESSAGE", ERROR: "ERROR",
    REQUEST_NAVIGATION: "REQUEST_NAVIGATION",
}

VERS_RENDERER = frozenset((CREATE_DOCUMENT, NAVIGATE, RESIZE, INPUT_EVENT,
                           TICK, SURFACE, CLOSE))
VERS_NAVIGATEUR = frozenset((READY, TITLE_CHANGED, URL_CHANGED, CURSOR_CHANGED,
                             FRAME_READY, CONSOLE_MESSAGE, ERROR,
                             REQUEST_NAVIGATION))


class Erreur(Exception):
    """Trame illisible. Toujours fatale pour la connexion, jamais ignoree."""


class Fin(Exception):
    """Le pair a ferme proprement. Ce n'est pas une erreur."""


def encode(genre, charge=None):
    """Fabrique une trame complete."""
    corps = b"" if charge is None else json.dumps(
        charge, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    if len(corps) > CHARGE_MAX:
        raise Erreur("charge de %d octets, maximum %d" % (len(corps), CHARGE_MAX))
    return EN_TETE.pack(VERSION, genre, len(corps)) + corps


class Canal:
    """Une prise, un tampon, et les descripteurs qui arrivent avec.

    ## Pourquoi ce n'est pas juste `recv`

    Deux raisons, et la seconde a coute une soiree.

    **Le decoupage.** `recv` rend ce qu'il a sous la main, pas ce qu'on lui
    demande : deux messages peuvent arriver dans le meme paquet, ou un seul en
    deux morceaux. Un lecteur qui prend ce qui vient pour un message entier
    fonctionne tant que les messages sont petits et la machine peu chargee,
    c'est-a-dire jusqu'au jour ou il faudrait qu'il fonctionne. Le tampon rend
    le decoupage du reseau invisible a l'appelant.

    **Les descripteurs.** Un descripteur envoye par `SCM_RIGHTS` voyage dans les
    donnees **auxiliaires** du meme envoi que les octets. Un `recv` ordinaire lit
    les octets et **jette silencieusement** le descripteur — le noyau le ferme,
    et personne n'est averti. C'est exactement ce qui est arrive ici : la surface
    partait, le message arrivait, et le renderer se retrouvait avec un message
    parlant d'une surface dont le descripteur n'existait plus.

    La seule facon sure est de lire par `recvmsg` **partout**, y compris pour les
    messages qui ne portent rien : on ne sait pas a l'avance lequel en portera.
    Les descripteurs recus s'accumulent dans `descripteurs`, ou le gestionnaire
    du message correspondant vient les chercher.

    ## Non bloquant

    Si la prise n'a pas de quoi completer un message, `lis` leve
    `BlockingIOError` (ou `socket.timeout`) **sans rien consommer** : le tampon
    reste tel quel et l'appel suivant reprend ou celui-ci s'est arrete. C'est ce
    qui permet au navigateur de sonder son renderer sans jamais se bloquer.
    """

    # Assez grand pour que la plupart des messages tiennent en une lecture,
    # assez petit pour ne pas immobiliser un tampon par renderer.
    LECTURE = 65536
    # Un envoi ne porte jamais plus d'un descripteur dans ce protocole : la
    # surface. En demander quatre laisse de la marge sans ouvrir la porte a un
    # pair qui en enverrait mille.
    DESCRIPTEURS_MAX = 4

    def __init__(self, prise):
        self.prise = prise
        self.tampon = bytearray()
        self.descripteurs = []

    def _assure(self, combien):
        """Garantit `combien` octets dans le tampon, ou leve."""
        while len(self.tampon) < combien:
            donnees, recus, drapeaux, _ = socket.recv_fds(
                self.prise, self.LECTURE, self.DESCRIPTEURS_MAX)
            if recus:
                self.descripteurs.extend(recus)
            if drapeaux & socket.MSG_CTRUNC:
                raise Erreur("descripteurs tronques a la reception")
            if not donnees:
                if not self.tampon:
                    raise Fin()
                raise Erreur("trame tronquee : %d octets en attente"
                             % len(self.tampon))
            self.tampon += donnees

    def lis(self, attendus=None):
        """Lit une trame complete. Rend `(genre, charge)`.

        `attendus` est l'ensemble des genres acceptables **dans ce sens**. Le
        preciser n'est pas une politesse : sans lui, un renderer compromis
        pourrait envoyer un `CLOSE` au navigateur, qui n'a aucune raison de
        savoir le lire.
        """
        self._assure(TAILLE_EN_TETE)
        version, genre, longueur = EN_TETE.unpack_from(self.tampon, 0)
        if version != VERSION:
            raise Erreur("version %d, attendue %d" % (version, VERSION))
        if longueur > CHARGE_MAX:
            raise Erreur("charge annoncee de %d octets, maximum %d"
                         % (longueur, CHARGE_MAX))
        if genre not in NOMS:
            raise Erreur("genre inconnu : %d" % genre)
        if attendus is not None and genre not in attendus:
            raise Erreur("genre %s inattendu dans ce sens" % NOMS[genre])

        self._assure(TAILLE_EN_TETE + longueur)
        corps = bytes(self.tampon[TAILLE_EN_TETE:TAILLE_EN_TETE + longueur])
        # Le message n'est retire du tampon qu'une fois complet : une lecture
        # interrompue au milieu ne fait donc rien perdre.
        del self.tampon[:TAILLE_EN_TETE + longueur]

        if not corps:
            return genre, None
        try:
            return genre, json.loads(corps.decode("utf-8"))
        except (UnicodeDecodeError, ValueError) as e:
            raise Erreur("charge de %s illisible : %s" % (NOMS[genre], e))

    def prends_descripteur(self):
        """Le prochain descripteur recu, ou `None`."""
        return self.descripteurs.pop(0) if self.descripteurs else None

    def envoie(self, genre, charge=None):
        self.prise.sendall(encode(genre, charge))

    def envoie_avec_descripteur(self, genre, charge, descripteur):
        """Envoie une trame **et** un descripteur, dans le meme envoi.

        Les deux ensemble, jamais l'un puis l'autre : c'est ce qui evite d'avoir
        a les reapparier a l'arrivee, et ce qui garantit que le message qui
        decrit la surface arrive avec la surface.
        """
        socket.send_fds(self.prise, [encode(genre, charge)], [descripteur])


def envoie(prise, genre, charge=None):
    prise.sendall(encode(genre, charge))
