"""Lecture audio et video : de la trame decodee a l'ecran et au haut-parleur.

`bomedia` decode et rend des trames horodatees ; ce module decide **quand** les
montrer et les jouer. C'est toute la difference entre un decodeur et un lecteur.

## L'horloge, c'est le son

L'image se cale sur le son, jamais l'inverse. Un son qui accelere ou ralentit
s'entend immediatement ; une image montree deux centiemes trop tot ou trop tard
ne se voit pas. La position de lecture se lit donc sur la carte — les octets
ecrits, moins ceux qui n'ont pas encore ete joues (`SNDCTL_DSP_GETODELAY`) — et
l'image en retard est sautee plutot que montree en retard.

Sans piste audio, l'horloge est celle du systeme, et le raisonnement est le
meme.

## Ce qui tourne quand

Rien ne s'execute en fond : `bat()` est appele par le battement du navigateur,
toutes les 16 ms. A chaque passage, le lecteur decode ce qu'il faut pour garder
un peu d'avance, pousse le son disponible vers `/dev/dsp`, et retient l'image a
afficher. Un lecteur qui n'est pas en lecture ne coute rien.
"""

import fcntl
import os
import struct
import time

import bo

try:
    import bomedia
    DISPONIBLE = True
except ImportError:  # hote sans libavcodec : les elements media restent inertes
    bomedia = None
    DISPONIBLE = False

# ioctls OSS dont le lecteur a besoin.
SNDCTL_DSP_RESET = 0x00005000
SNDCTL_DSP_SPEED = 0xC0045002
SNDCTL_DSP_SETFMT = 0xC0045005
SNDCTL_DSP_CHANNELS = 0xC0045006
SNDCTL_DSP_GETODELAY = 0x80045017
AFMT_S16_LE = 0x00000010

# Format que `bomedia` produit, et que la carte accepte.
FREQUENCE = 48000
VOIES = 2
OCTETS_PAR_TRAME = VOIES * 2

# Avance visee dans la file audio : assez pour couvrir une saccade de decodage,
# assez peu pour qu'une pause s'entende tout de suite.
AVANCE_MS = 300

# Au-dela, on cesse de decoder : une video entierement decodee en memoire
# tiendrait des centaines de mega-octets.
IMAGES_EN_ATTENTE_MAX = 8


class SortieAudio:
    """`/dev/dsp`, ouvert une fois et partage par tous les lecteurs.

    Le materiel n'a qu'une sortie : deux lecteurs qui joueraient ensemble se
    marcheraient dessus. Le premier qui demarre la prend, les autres restent
    muets — ce qui vaut mieux qu'un melange qu'on ne sait pas faire.
    """

    _instance = None

    def __init__(self):
        self.fd = None
        self.proprietaire = None
        self.octets_ecrits = 0

    @classmethod
    def partagee(cls):
        if cls._instance is None:
            cls._instance = cls()
        return cls._instance

    def _ouvre(self):
        if self.fd is not None:
            return True
        try:
            self.fd = os.open("/dev/dsp", os.O_WRONLY)
        except OSError:
            self.fd = None
            return False
        try:
            for requete, valeur in ((SNDCTL_DSP_SETFMT, AFMT_S16_LE),
                                    (SNDCTL_DSP_CHANNELS, VOIES),
                                    (SNDCTL_DSP_SPEED, FREQUENCE)):
                fcntl.ioctl(self.fd, requete, struct.pack("i", valeur), True)
        except OSError:
            # Le pilote refuse le reglage : on ferme plutot que de jouer faux.
            os.close(self.fd)
            self.fd = None
            return False
        return True

    def prend(self, lecteur):
        if not self._ouvre():
            return False
        if self.proprietaire is None or self.proprietaire is lecteur:
            self.proprietaire = lecteur
            return True
        return False

    def rend(self, lecteur):
        if self.proprietaire is lecteur:
            self.vide()
            self.proprietaire = None

    def ecrit(self, lecteur, donnees):
        """Pousse du PCM. Rend le nombre d'octets consommes."""
        if self.proprietaire is not lecteur or self.fd is None or not donnees:
            return 0
        try:
            ecrits = os.write(self.fd, donnees)
        except OSError:
            return 0
        self.octets_ecrits += ecrits
        return ecrits

    def retard_octets(self):
        """Octets ecrits mais pas encore joues."""
        if self.fd is None:
            return 0
        try:
            reponse = fcntl.ioctl(self.fd, SNDCTL_DSP_GETODELAY,
                                  struct.pack("i", 0), True)
            return struct.unpack("i", reponse)[0]
        except OSError:
            return 0

    def position_secondes(self, lecteur):
        """Position reellement jouee, en secondes."""
        if self.proprietaire is not lecteur or self.fd is None:
            return None
        joues = self.octets_ecrits - self.retard_octets()
        if joues < 0:
            joues = 0
        return joues / float(FREQUENCE * OCTETS_PAR_TRAME)

    def vide(self):
        if self.fd is None:
            return
        try:
            fcntl.ioctl(self.fd, SNDCTL_DSP_RESET, 0)
        except OSError:
            pass
        self.octets_ecrits = 0


class Lecteur:
    """Un `<video>` ou un `<audio>` : son flux, son horloge, son image."""

    def __init__(self, largeur=0, hauteur=0, journal=None):
        self.journal = journal or (lambda niveau, texte: None)
        self.source = None
        self.info = {}
        self.largeur = largeur
        self.hauteur = hauteur

        self.en_lecture = False
        self.termine = False
        self.boucle = False
        self.volume = 1.0
        self.muet = False

        # Image courante : identifiant dans le cache de l'hote, reutilise d'une
        # trame a l'autre.
        self.image = None
        self.image_horodatage = 0.0

        self._images_en_attente = []
        self._pcm_en_attente = b""
        self._depart_horloge = None
        self._position_sans_son = 0.0
        self._octets_pousses = 0
        self._sortie = SortieAudio.partagee()
        self._donnees = b""

    # --- Chargement ----------------------------------------------------------

    def charge(self, donnees):
        """Ouvre un flux complet. Rend `True` si le format est reconnu."""
        if not DISPONIBLE:
            self.journal("warn", "media: libavcodec absent")
            return False
        self.ferme()
        self._donnees = bytes(donnees)
        try:
            self.source = bomedia.ouvre(self._donnees, self.largeur, self.hauteur)
        except Exception as e:  # noqa: BLE001 — un media casse n'arrete pas la page
            self.journal("warn", "media: %s" % e)
            self.source = None
            return False
        self.info = bomedia.info(self.source)
        self.largeur = self.info.get("largeur", self.largeur)
        self.hauteur = self.info.get("hauteur", self.hauteur)
        self.termine = False
        return True

    def alimente(self, donnees):
        """Ajoute un segment — c'est le chemin des Media Source Extensions.

        Tant que le flux n'a pas pu etre ouvert (l'en-tete d'initialisation
        n'est pas encore complet), on accumule et on retente a chaque ajout.
        """
        if not DISPONIBLE:
            return False
        self._donnees += bytes(donnees)
        if self.source is None:
            return self.charge(self._donnees)
        try:
            bomedia.alimente(self.source, bytes(donnees))
        except Exception as e:  # noqa: BLE001
            self.journal("warn", "media: alimentation refusee : %s" % e)
            return False
        return True

    def ferme(self):
        if self.source is not None:
            self._sortie.rend(self)
            try:
                bomedia.ferme(self.source)
            except Exception:  # noqa: BLE001
                pass
            self.source = None
        self._images_en_attente = []
        self._pcm_en_attente = b""
        self.en_lecture = False

    # --- Commandes -----------------------------------------------------------

    def joue(self):
        if self.source is None:
            return
        if self.termine:
            self.cherche(0.0)
        self.en_lecture = True
        self._depart_horloge = time.monotonic() - self._position_sans_son
        if self.info.get("a_audio"):
            self._sortie.prend(self)

    def pause(self):
        if not self.en_lecture:
            return
        self._position_sans_son = self.position()
        self.en_lecture = False
        self._sortie.rend(self)

    def cherche(self, seconde):
        if self.source is None:
            return
        try:
            bomedia.cherche(self.source, max(0.0, seconde))
        except Exception:  # noqa: BLE001
            return
        self._images_en_attente = []
        self._pcm_en_attente = b""
        self._sortie.vide()
        self._octets_pousses = 0
        self._position_sans_son = max(0.0, seconde)
        self._depart_horloge = time.monotonic() - self._position_sans_son
        self.termine = False

    def duree(self):
        return float(self.info.get("duree", 0.0) or 0.0)

    def position(self):
        """Position de lecture, en secondes."""
        if self.source is None:
            return 0.0
        if not self.en_lecture:
            return self._position_sans_son
        depuis_le_son = self._sortie.position_secondes(self)
        if depuis_le_son is not None:
            return depuis_le_son
        if self._depart_horloge is None:
            return self._position_sans_son
        return time.monotonic() - self._depart_horloge

    # --- Battement -----------------------------------------------------------

    def bat(self):
        """Decode, pousse le son, choisit l'image. Rend `True` si l'image change."""
        if self.source is None or not self.en_lecture:
            return False

        self._remplit()
        self._pousse_son()
        return self._choisit_image()

    def _remplit(self):
        """Decode jusqu'a avoir assez d'avance, sans jamais tout decoder."""
        cible = int(FREQUENCE * OCTETS_PAR_TRAME * AVANCE_MS / 1000.0)
        gardes = 0
        while (len(self._pcm_en_attente) < cible
               and len(self._images_en_attente) < IMAGES_EN_ATTENTE_MAX
               and gardes < 64):
            gardes += 1
            try:
                trame = bomedia.trame(self.source)
            except Exception as e:  # noqa: BLE001
                self.journal("warn", "media: decodage interrompu : %s" % e)
                trame = None
            if trame is None:
                if not self._images_en_attente and not self._pcm_en_attente:
                    self._fin()
                return
            if trame["type"] == "audio":
                self._pcm_en_attente += self._applique_volume(trame["donnees"])
            else:
                self._images_en_attente.append(
                    (trame["horodatage"], trame["donnees"],
                     trame["largeur"], trame["hauteur"]))
            # Sans piste audio, la boucle n'aurait pas de critere d'arret par le
            # PCM : les images suffisent.
            if not self.info.get("a_audio") and self._images_en_attente:
                return

    def _applique_volume(self, pcm):
        """Attenue le PCM. Le materiel n'a pas de reglage par flux."""
        if self.muet:
            return b"\x00" * len(pcm)
        if self.volume >= 0.999:
            return pcm
        facteur = int(max(0.0, min(1.0, self.volume)) * 256)
        if facteur == 0:
            return b"\x00" * len(pcm)
        echantillons = memoryview(pcm).cast("h")
        attenues = bytearray(len(pcm))
        vue = memoryview(attenues).cast("h")
        for i, valeur in enumerate(echantillons):
            vue[i] = (valeur * facteur) >> 8
        return bytes(attenues)

    def _pousse_son(self):
        if not self._pcm_en_attente:
            return
        if not self.info.get("a_audio"):
            return
        if not self._sortie.prend(self):
            # Un autre lecteur tient la carte : on avance quand meme, en muet.
            self._pcm_en_attente = b""
            return
        ecrits = self._sortie.ecrit(self, self._pcm_en_attente)
        if ecrits > 0:
            self._pcm_en_attente = self._pcm_en_attente[ecrits:]
            self._octets_pousses += ecrits

    def _choisit_image(self):
        """Retient la derniere image dont l'heure est passee. Saute les autres."""
        if not self._images_en_attente:
            return False
        maintenant = self.position()
        choisie = None
        while self._images_en_attente:
            horodatage, donnees, largeur, hauteur = self._images_en_attente[0]
            if horodatage > maintenant + 0.001 and choisie is not None:
                break
            if horodatage > maintenant + 0.001 and choisie is None:
                # Rien n'est encore du : on garde l'image courante.
                if self.image is not None:
                    return False
                # Sauf s'il n'y en a aucune — la premiere s'affiche tout de suite.
            choisie = self._images_en_attente.pop(0)
            if choisie[0] > maintenant + 0.001:
                break

        if choisie is None:
            return False
        horodatage, donnees, largeur, hauteur = choisie
        try:
            identifiant = -1 if self.image is None else self.image
            resultat = bo.image_brute(donnees, largeur, hauteur, identifiant)
        except Exception as e:  # noqa: BLE001
            self.journal("warn", "media: image refusee : %s" % e)
            return False
        self.image = resultat[0]
        self.largeur, self.hauteur = resultat[1], resultat[2]
        self.image_horodatage = horodatage
        return True

    def _fin(self):
        if self.boucle:
            self.cherche(0.0)
            return
        self.termine = True
        self.en_lecture = False
        self._position_sans_son = self.duree()
        self._sortie.rend(self)


def codecs():
    """Decodeurs disponibles, pour `canPlayType`."""
    if not DISPONIBLE:
        return []
    try:
        return bomedia.codecs()
    except Exception:  # noqa: BLE001
        return []


def sait_lire(type_mime):
    """Repond comme `HTMLMediaElement.canPlayType` : '', 'maybe' ou 'probably'.

    Le type MIME peut nommer ses codecs (`video/mp4; codecs="avc1.42E01E"`).
    Quand il le fait, on verifie ce qu'il nomme ; sinon on repond sur le seul
    conteneur, d'ou la nuance entre « peut-etre » et « probablement » que la
    norme prevoit precisement pour ce cas.
    """
    if not DISPONIBLE or not type_mime:
        return ""
    disponibles = set(codecs())
    texte = type_mime.lower()
    conteneur = texte.split(";")[0].strip()

    connus = {
        "video/mp4": "h264", "audio/mp4": "aac", "video/webm": "vp9",
        "audio/webm": "opus", "video/x-msvideo": "mjpeg", "audio/wav": "pcm_s16le",
        "audio/wave": "pcm_s16le", "audio/x-wav": "pcm_s16le",
        "audio/mpeg": "mp3", "audio/ogg": "vorbis", "video/ogg": "theora",
        "audio/aac": "aac", "audio/flac": "flac", "audio/opus": "opus",
    }
    if conteneur not in connus:
        return ""
    if connus[conteneur] not in disponibles:
        return ""

    if "codecs=" in texte:
        nommes = texte.split("codecs=")[1].strip().strip('"\'')
        for codec in nommes.split(","):
            codec = codec.strip()
            racine = {"avc1": "h264", "avc3": "h264", "mp4a": "aac",
                      "vp8": "vp8", "vp09": "vp9", "vp9": "vp9",
                      "opus": "opus", "vorbis": "vorbis"}.get(codec.split(".")[0])
            if racine is None or racine not in disponibles:
                return ""
        return "probably"
    return "maybe"
