"""Le fil entre le navigateur et son processus de rendu.

Le canal de controle est un flux SOCK_STREAM. Une trame peut donc etre
decoupee en plusieurs ecritures/lectures, et une socket non bloquante peut
n'accepter qu'une partie d'une trame avant de rendre EAGAIN.

Invariant important :

    une trame encodee une fois n'est jamais reconstruite apres une
    ecriture partielle.

Les octets non encore transmis restent dans `sortie` jusqu'a ce que le pair
soit de nouveau capable de les recevoir.
"""

import errno
import json
import socket
import struct


VERSION = 1

# En-tete :
#   version u16
#   genre   u16
#   longueur u32
EN_TETE = struct.Struct("!HHI")
TAILLE_EN_TETE = EN_TETE.size

# Une trame de controle qui depasse cela est une erreur.
CHARGE_MAX = 1 << 20

# On borne egalement la file de sortie.
#
# Les gros corps HTTP ne passent pas dans une seule trame :
# ils sont decoupes avec MORCEAU_FETCH.
#
# 8 MiB laisse donc une marge confortable tout en empechant un renderer
# bloque de faire grossir indefiniment la memoire du navigateur.
SORTIE_MAX = 8 << 20


# --- Navigateur -> Renderer ---------------------------------------------------

CREATE_DOCUMENT = 1
NAVIGATE = 2
RESIZE = 3
INPUT_EVENT = 4
TICK = 5
SURFACE = 6
CLOSE = 7
FETCH_RESPONSE = 8
FETCH_DATA = 9
AUDIT = 10


# --- Renderer -> Navigateur ---------------------------------------------------

READY = 128
TITLE_CHANGED = 129
URL_CHANGED = 130
CURSOR_CHANGED = 131
FRAME_READY = 132
CONSOLE_MESSAGE = 133
ERROR = 134
REQUEST_NAVIGATION = 135
FOCUS_CHANGED = 136
FETCH_REQUEST = 137
AUDIT_RESULT = 138


NOMS = {
    CREATE_DOCUMENT: "CREATE_DOCUMENT",
    NAVIGATE: "NAVIGATE",
    RESIZE: "RESIZE",
    INPUT_EVENT: "INPUT_EVENT",
    TICK: "TICK",
    SURFACE: "SURFACE",
    CLOSE: "CLOSE",
    FETCH_RESPONSE: "FETCH_RESPONSE",
    FETCH_DATA: "FETCH_DATA",
    AUDIT: "AUDIT",

    READY: "READY",
    TITLE_CHANGED: "TITLE_CHANGED",
    URL_CHANGED: "URL_CHANGED",
    CURSOR_CHANGED: "CURSOR_CHANGED",
    FRAME_READY: "FRAME_READY",
    CONSOLE_MESSAGE: "CONSOLE_MESSAGE",
    ERROR: "ERROR",
    REQUEST_NAVIGATION: "REQUEST_NAVIGATION",
    FOCUS_CHANGED: "FOCUS_CHANGED",
    FETCH_REQUEST: "FETCH_REQUEST",
    AUDIT_RESULT: "AUDIT_RESULT",
}


VERS_RENDERER = frozenset((
    CREATE_DOCUMENT,
    NAVIGATE,
    RESIZE,
    INPUT_EVENT,
    TICK,
    SURFACE,
    CLOSE,
    FETCH_RESPONSE,
    FETCH_DATA,
    AUDIT,
))


VERS_NAVIGATEUR = frozenset((
    READY,
    TITLE_CHANGED,
    URL_CHANGED,
    CURSOR_CHANGED,
    FRAME_READY,
    CONSOLE_MESSAGE,
    ERROR,
    REQUEST_NAVIGATION,
    FOCUS_CHANGED,
    FETCH_REQUEST,
    AUDIT_RESULT,
))


# Corps des ressources courtees.
#
# Base64 grossit d'environ un tiers. 192 Kio garde donc la trame encodee
# tres largement sous CHARGE_MAX.
MORCEAU_FETCH = 192 * 1024


class Erreur(Exception):
    """Trame/protocole invalide."""


class Fin(Exception):
    """Le pair a ferme proprement."""


def encode(genre, charge=None):
    """Fabrique une trame complete et immutable."""

    corps = (
        b""
        if charge is None
        else json.dumps(
            charge,
            ensure_ascii=False,
            separators=(",", ":"),
        ).encode("utf-8")
    )

    if len(corps) > CHARGE_MAX:
        raise Erreur(
            "charge de %d octets, maximum %d"
            % (len(corps), CHARGE_MAX)
        )

    return EN_TETE.pack(
        VERSION,
        genre,
        len(corps),
    ) + corps


class Canal:
    """Canal bidirectionnel encadre au-dessus d'un SOCK_STREAM.

    Deux tampons sont distincts :

    `tampon`
        octets deja recus mais pas encore consommes comme trame complete.

    `sortie`
        octets deja acceptes par le protocole mais pas encore acceptes
        integralement par le noyau.

    Cette seconde file est indispensable avec les sockets non bloquantes.

    `send()` peut par exemple faire :

        trame de 100 octets
        send -> 37
        send -> EAGAIN

    Les 63 octets restants doivent alors rester exactement tels quels.
    Reencoder ou abandonner la trame corromprait le flux.
    """

    LECTURE = 65536
    DESCRIPTEURS_MAX = 4

    def __init__(self, prise):
        self.prise = prise

        # Reception.
        self.tampon = bytearray()
        self.descripteurs = []

        # Emission.
        self.sortie = bytearray()

    # ------------------------------------------------------------------
    # Reception
    # ------------------------------------------------------------------

    def _assure(self, combien):
        """Garantit `combien` octets dans le tampon de reception."""

        while len(self.tampon) < combien:
            donnees, recus, drapeaux, _ = socket.recv_fds(
                self.prise,
                self.LECTURE,
                self.DESCRIPTEURS_MAX,
            )

            if recus:
                self.descripteurs.extend(recus)

            if drapeaux & socket.MSG_CTRUNC:
                raise Erreur(
                    "descripteurs tronques a la reception"
                )

            if not donnees:
                if not self.tampon:
                    raise Fin()

                raise Erreur(
                    "trame tronquee : %d octets en attente"
                    % len(self.tampon)
                )

            self.tampon += donnees

    def lis(self, attendus=None):
        """Lit une trame complete.

        Rend :

            (genre, charge)

        Une lecture non bloquante incomplete peut lever BlockingIOError.
        Aucun octet deja recu n'est perdu.
        """

        self._assure(TAILLE_EN_TETE)

        version, genre, longueur = EN_TETE.unpack_from(
            self.tampon,
            0,
        )

        if version != VERSION:
            raise Erreur(
                "version %d, attendue %d"
                % (version, VERSION)
            )

        if longueur > CHARGE_MAX:
            raise Erreur(
                "charge annoncee de %d octets, maximum %d"
                % (longueur, CHARGE_MAX)
            )

        if genre not in NOMS:
            raise Erreur(
                "genre inconnu : %d" % genre
            )

        if attendus is not None and genre not in attendus:
            raise Erreur(
                "genre %s inattendu dans ce sens"
                % NOMS[genre]
            )

        total = TAILLE_EN_TETE + longueur

        self._assure(total)

        corps = bytes(
            self.tampon[
                TAILLE_EN_TETE:total
            ]
        )

        # Retirer uniquement quand la trame entiere est presente.
        del self.tampon[:total]

        if not corps:
            return genre, None

        try:
            charge = json.loads(
                corps.decode("utf-8")
            )
        except (UnicodeDecodeError, ValueError) as e:
            raise Erreur(
                "charge de %s illisible : %s"
                % (NOMS[genre], e)
            )

        return genre, charge

    def prends_descripteur(self):
        """Rend le prochain fd SCM_RIGHTS recu."""

        if not self.descripteurs:
            return None

        return self.descripteurs.pop(0)

    # ------------------------------------------------------------------
    # Emission
    # ------------------------------------------------------------------

    def octets_en_attente(self):
        """Nombre d'octets pas encore acceptes par le noyau."""

        return len(self.sortie)

    def a_envoyer(self):
        """Y a-t-il encore quelque chose en attente ?"""

        return bool(self.sortie)

    def vidange(self):
        """Tente d'envoyer tout ce qui reste dans `sortie`.

        Retourne :

            True
                tout a ete transmis.

            False
                la socket non bloquante a rendu EAGAIN.

        EAGAIN n'est PAS une erreur de protocole.
        C'est la contre-pression normale du socketpair.
        """

        while self.sortie:
            try:
                envoyes = self.prise.send(self.sortie)

            except BlockingIOError:
                return False

            except InterruptedError:
                # EINTR : rien n'est perdu, on recommence.
                continue

            if envoyes == 0:
                raise Fin()

            del self.sortie[:envoyes]

        return True

    def envoie(self, genre, charge=None):
        """Ajoute une trame a la file et tente de la transmettre.

        Important :
        la trame est encodee UNE SEULE FOIS.

        Une ecriture partielle n'est donc jamais reencodee.
        """

        trame = encode(genre, charge)

        nouvelle_taille = (
            len(self.sortie)
            + len(trame)
        )

        if nouvelle_taille > SORTIE_MAX:
            raise Erreur(
                "file de sortie saturee : %d octets, maximum %d"
                % (nouvelle_taille, SORTIE_MAX)
            )

        self.sortie.extend(trame)

        return self.vidange()

    def envoie_avec_descripteur(
        self,
        genre,
        charge,
        descripteur,
    ):
        """Envoie une trame avec un fd SCM_RIGHTS.

        Cette operation doit normalement etre executee lorsque la socket
        est temporairement bloquante.

        Avant le sendmsg on vide les trames ordinaires deja en attente afin
        de conserver strictement l'ordre du protocole.
        """

        if self.sortie:
            if not self.vidange():
                raise BlockingIOError(
                    errno.EAGAIN,
                    "canal encore sature avant SCM_RIGHTS",
                )

        trame = encode(
            genre,
            charge,
        )

        try:
            envoyes = socket.send_fds(
                self.prise,
                [trame],
                [descripteur],
            )

        except InterruptedError:
            # Aucun moyen fiable de savoir si les donnees auxiliaires ont
            # deja ete attachees apres EINTR : on remonte plutot que de
            # risquer d'envoyer deux fois le fd.
            raise

        if envoyes <= 0:
            raise Fin()

        # SOCK_STREAM peut accepter une partie seulement de la trame.
        #
        # Le descripteur a deja accompagne le premier fragment.
        # Le reste est donc de simples octets et peut rejoindre la file
        # de sortie normale.
        if envoyes < len(trame):
            reste = trame[envoyes:]

            if len(self.sortie) + len(reste) > SORTIE_MAX:
                raise Erreur(
                    "file de sortie saturee apres SCM_RIGHTS"
                )

            self.sortie.extend(reste)

            return self.vidange()

        return True


def envoie(prise, genre, charge=None):
    """Compatibilite pour les rares appelants qui n'utilisent pas Canal.

    Cette fonction reste bloquante et ne doit pas etre employee sur la
    socket non bloquante du superviseur.
    """

    prise.sendall(
        encode(
            genre,
            charge,
        )
    )