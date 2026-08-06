"""YouTube : extraire le flux plutot qu'executer l'application.

## Pourquoi on n'execute pas la page de YouTube

La page de lecture de YouTube est une application : deux megaoctets de
JavaScript, Polymer, des composants a portee isolee, des travailleurs de
service, et un lecteur qui alimente son `<video>` par segments DASH. La faire
tourner demanderait d'implementer une fraction enorme d'un navigateur moderne —
et, meme implementee, elle ne tournerait pas a une vitesse utilisable sous
emulation.

Ce module fait ce que font les clients contraints — NewPipe, les lecteurs
embarques, les televiseurs : il demande a YouTube **ou est le flux**, et le lit
nativement. Les flux, eux, sont de simples ressources HTTP en H.264 ou VP9, que
`bomedia` sait decoder depuis qu'il porte libavcodec.

## Comment

1. l'identifiant de la video est tire de l'adresse ;
2. la reponse du lecteur est demandee a l'API interne (`/youtubei/v1/player`),
   avec un contexte de client leger ; a defaut, elle est extraite de la page de
   lecture (`ytInitialPlayerResponse`) ;
3. le format est choisi parmi ceux que la machine sait vraiment decoder, en
   privilegiant les flux **progressifs** — image et son dans un seul fichier,
   donc sans DASH ni Media Source Extensions ;
4. si l'adresse arrive signee, la signature est dechiffree en executant la
   fonction de `base.js` dans QuickJS (voir [`signature`]) ;
5. une page de lecture propre est batie, avec un `<video>` qui pointe sur
   l'adresse resolue.

## Ce que cela ne fait pas

Ni recommandations, ni commentaires, ni connexion a un compte, ni contenu
protege par DRM — celui-la exige un module proprietaire (Widevine) qui ne
s'implemente pas. La lecture ordinaire, elle, n'est pas chiffree.
"""

import json
import re
import urllib.parse

from . import reseau

# Clef publique du client web de YouTube. Elle figure en clair dans leur page
# et n'authentifie personne : c'est un simple jeton de routage d'API.
CLEF_INNERTUBE = "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8"

API = "https://www.youtube.com/youtubei/v1/player"

# Contextes de client essayes dans l'ordre.
#
# L'ordre n'est pas indifferent, et c'est meme tout l'enjeu. YouTube exige
# depuis 2024 une **preuve d'origine** (`poToken`) de ses clients web : sans
# elle, la reponse est un refus poli — « confirmez que vous n'etes pas un
# robot » — ou des adresses qui rendent 403. Cette preuve se fabrique en
# executant du code d'attestation que nous ne pouvons pas faire tourner.
#
# Restent les clients qui n'en demandent pas. `ANDROID_VR` est le plus large :
# il ne reclame ni preuve d'origine ni attestation d'application, et rend des
# adresses non signees. Le client de television le suit, puis les clients
# mobiles ordinaires, et le web en tout dernier — il fonctionne encore pour les
# videos anciennes, et un refus coute une requete, pas la lecture.
CLIENTS = (
    {
        "nom": "ANDROID_VR",
        "contexte": {
            "clientName": "ANDROID_VR",
            "clientVersion": "1.60.19",
            "deviceMake": "Oculus",
            "deviceModel": "Quest 3",
            "osName": "Android",
            "osVersion": "12L",
            "androidSdkVersion": 32,
            "hl": "fr", "gl": "FR",
        },
        "agent": "com.google.android.apps.youtube.vr.oculus/1.60.19 "
                 "(Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip",
    },
    {
        "nom": "IOS",
        "contexte": {
            "clientName": "IOS",
            "clientVersion": "19.29.1",
            "deviceModel": "iPhone16,2",
            "hl": "fr", "gl": "FR",
        },
        "agent": "com.google.ios.youtube/19.29.1 (iPhone16,2; U; CPU iOS 17_5_1 like Mac OS X)",
    },
    {
        "nom": "ANDROID",
        "contexte": {
            "clientName": "ANDROID",
            "clientVersion": "19.29.37",
            "androidSdkVersion": 34,
            "hl": "fr", "gl": "FR",
        },
        "agent": "com.google.android.youtube/19.29.37 (Linux; U; Android 14) gzip",
    },
    {
        "nom": "TVHTML5_SIMPLY_EMBEDDED_PLAYER",
        "contexte": {
            "clientName": "TVHTML5_SIMPLY_EMBEDDED_PLAYER",
            "clientVersion": "2.0",
            "hl": "fr", "gl": "FR",
        },
        "agent": "Mozilla/5.0 (PlayStation; PlayStation 4/12.00) AppleWebKit/605.1.15",
    },
    {
        "nom": "WEB",
        "contexte": {
            "clientName": "WEB",
            "clientVersion": "2.20240726.00.00",
            "hl": "fr", "gl": "FR",
        },
        "agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36",
    },
)

# Les `itag` progressifs : image et son dans un seul fichier. Ce sont eux qu'on
# cherche en premier — pas de DASH, pas de deux flux a synchroniser.
PROGRESSIFS = {
    18: ("mp4", "h264", "aac", 360),
    22: ("mp4", "h264", "aac", 720),
    59: ("mp4", "h264", "aac", 480),
    17: ("3gp", "mpeg4", "aac", 144),
    36: ("3gp", "mpeg4", "aac", 240),
}

# Au-dela, une machine emulee sans acceleration ne suit plus. Ce n'est pas une
# limite du decodeur mais du debit : mieux vaut une image fluide en 360p qu'une
# image saccadee en 1080p.
HAUTEUR_MAX_PAR_DEFAUT = 480


def est_youtube(url):
    """L'adresse designe-t-elle une video YouTube ?"""
    return identifiant(url) is not None


def identifiant(url):
    """Identifiant de la video, ou `None` si l'adresse n'en designe pas une."""
    if not url:
        return None
    morceaux = urllib.parse.urlsplit(url)
    hote = (morceaux.hostname or "").lower()
    if hote.startswith("www."):
        hote = hote[4:]
    if hote.startswith("m."):
        hote = hote[2:]

    if hote == "youtu.be":
        candidat = morceaux.path.lstrip("/").split("/")[0]
        return candidat if _identifiant_valide(candidat) else None

    if hote not in ("youtube.com", "youtube-nocookie.com", "music.youtube.com"):
        return None

    if morceaux.path == "/watch":
        parametres = urllib.parse.parse_qs(morceaux.query)
        candidat = (parametres.get("v") or [""])[0]
        return candidat if _identifiant_valide(candidat) else None

    for prefixe in ("/embed/", "/v/", "/shorts/", "/live/"):
        if morceaux.path.startswith(prefixe):
            candidat = morceaux.path[len(prefixe):].split("/")[0]
            return candidat if _identifiant_valide(candidat) else None
    return None


def _identifiant_valide(candidat):
    return bool(candidat) and len(candidat) == 11 and re.fullmatch(
        r"[A-Za-z0-9_-]{11}", candidat) is not None


# --- Reponse du lecteur -------------------------------------------------------

def reponse_lecteur(video, journal=None):
    """Interroge YouTube et rend `(donnees, nom du client)`.

    Chaque client est essaye a son tour : le premier qui rend un flux jouable
    gagne. C'est necessaire parce que YouTube refuse regulierement tel ou tel
    contexte, et qu'un client refuse rend une reponse d'apparence valide mais
    sans `streamingData`.
    """
    journal = journal or (lambda niveau, texte: None)
    derniere = None

    for client in CLIENTS:
        corps = json.dumps({
            "context": {"client": client["contexte"]},
            "videoId": video,
            "contentCheckOk": True,
            "racyCheckOk": True,
        })
        entetes = {
            "Content-Type": "application/json",
            "User-Agent": client["agent"],
            "Accept": "application/json",
            "X-Youtube-Client-Name": _numero_client(client["nom"]),
            "X-Youtube-Client-Version": client["contexte"]["clientVersion"],
            "Origin": "https://www.youtube.com",
        }
        adresse = "%s?key=%s&prettyPrint=false" % (API, CLEF_INNERTUBE)
        reponse = reseau.charge(adresse, methode="POST", corps=corps,
                                entetes=entetes, brut=True)
        if reponse.code != 200:
            journal("warn", "youtube: client %s -> code %s"
                    % (client["nom"], reponse.code))
            continue
        try:
            donnees = json.loads(reponse.octets.decode("utf-8", "replace"))
        except ValueError as e:
            journal("warn", "youtube: reponse illisible (%s) : %s"
                    % (client["nom"], e))
            continue

        statut = (donnees.get("playabilityStatus") or {}).get("status", "")
        if statut and statut != "OK":
            raison = (donnees.get("playabilityStatus") or {}).get("reason", statut)
            # Le refus par preuve d'origine se reconnait a son texte : le dire
            # tel quel evite de chercher une panne la ou il n'y en a pas.
            if _est_preuve_exigee(raison):
                raison = ("%s — ce client exige une preuve d'origine (poToken)"
                          % raison)
            journal("warn", "youtube: %s refuse (%s)" % (client["nom"], raison))
            derniere = raison
            continue
        if not donnees.get("streamingData"):
            journal("warn", "youtube: %s ne rend aucun flux" % client["nom"])
            continue
        journal("log", "youtube: reponse obtenue par le client %s" % client["nom"])
        return donnees, client["nom"]

    # Dernier recours : la page de lecture porte la meme structure en clair.
    donnees = _depuis_la_page(video, journal)
    if donnees is not None:
        return donnees, "PAGE"
    raise LookupError(derniere or "aucun client n'a rendu de flux")


def agent_du_client(nom):
    """L'agent utilisateur du client nomme, ou celui du web a defaut.

    Il compte au-dela de l'extraction : Google lie l'adresse d'un flux au client
    qui l'a demandee, et la meme adresse reclamee sous un autre agent rend 403.
    """
    for client in CLIENTS:
        if client["nom"] == nom:
            return client["agent"]
    return CLIENTS[-1]["agent"]


def _numero_client(nom):
    return {"WEB": "1", "ANDROID": "3", "IOS": "5",
            "TVHTML5_SIMPLY_EMBEDDED_PLAYER": "85",
            "ANDROID_VR": "28"}.get(nom, "1")


# Textes par lesquels YouTube signale qu'il attend une preuve d'origine.
_REFUS_PREUVE = ("confirm you", "not a bot", "sign in to confirm",
                 "connectez-vous pour confirmer")


def _est_preuve_exigee(raison):
    minuscule = (raison or "").lower()
    return any(marque in minuscule for marque in _REFUS_PREUVE)


def _depuis_la_page(video, journal):
    """Extrait `ytInitialPlayerResponse` de la page de lecture."""
    adresse = "https://www.youtube.com/watch?v=%s&hl=fr" % video
    reponse = reseau.charge(adresse, brut=True)
    if reponse.code != 200:
        return None
    texte = reponse.octets.decode("utf-8", "replace")
    donnees = _objet_apres(texte, "ytInitialPlayerResponse")
    if donnees is None:
        journal("warn", "youtube: ytInitialPlayerResponse absent de la page")
    return donnees


def _objet_apres(texte, nom):
    """Lit l'objet JSON qui suit `nom =` dans un script, en comptant les accolades.

    Une expression rationnelle ne peut pas faire cela : l'objet contient des
    accolades imbriquees et des chaines qui en contiennent aussi. On compte donc
    les niveaux, en sautant ce qui est entre guillemets.
    """
    depart = texte.find(nom)
    if depart < 0:
        return None
    debut = texte.find("{", depart)
    if debut < 0:
        return None

    niveau = 0
    dans_chaine = False
    echappe = False
    for position in range(debut, len(texte)):
        caractere = texte[position]
        if dans_chaine:
            if echappe:
                echappe = False
            elif caractere == "\\":
                echappe = True
            elif caractere == '"':
                dans_chaine = False
            continue
        if caractere == '"':
            dans_chaine = True
        elif caractere == "{":
            niveau += 1
        elif caractere == "}":
            niveau -= 1
            if niveau == 0:
                try:
                    return json.loads(texte[debut:position + 1])
                except ValueError:
                    return None
    return None


# --- Choix du format ----------------------------------------------------------

class Flux:
    """Un format retenu : son adresse, ce qu'il contient, ce qu'il pese."""

    __slots__ = ("url", "itag", "conteneur", "codec_video", "codec_audio",
                 "hauteur", "octets", "progressif", "signature_a_dechiffrer")

    def __init__(self, **champs):
        for nom in self.__slots__:
            setattr(self, nom, champs.get(nom))

    def __repr__(self):
        return "<flux itag %s %sp %s+%s>" % (
            self.itag, self.hauteur, self.codec_video, self.codec_audio)


def choisit_flux(donnees, hauteur_max=HAUTEUR_MAX_PAR_DEFAUT, codecs=None):
    """Choisit le meilleur flux jouable. Rend `(video, audio_separe_ou_None)`.

    Le progressif est prefere : image et son dans un fichier, donc rien a
    synchroniser et pas de DASH. A defaut, on prend la meilleure piste image
    sous la hauteur demandee et la meilleure piste son, que le lecteur jouera
    ensemble.
    """
    flux = donnees.get("streamingData") or {}
    disponibles = codecs if codecs is not None else ("h264", "vp9", "vp8", "aac",
                                                     "opus", "vorbis", "mpeg4")

    # 1. Progressif.
    candidats = []
    for format_ in flux.get("formats") or []:
        entree = _decrit(format_)
        if entree is None or not entree.progressif:
            continue
        if entree.hauteur and entree.hauteur > hauteur_max:
            continue
        if entree.codec_video not in disponibles:
            continue
        candidats.append(entree)
    if candidats:
        candidats.sort(key=lambda f: (f.hauteur or 0), reverse=True)
        return candidats[0], None

    # 2. Adaptatif : deux pistes a jouer ensemble.
    images, sons = [], []
    for format_ in flux.get("adaptiveFormats") or []:
        entree = _decrit(format_)
        if entree is None:
            continue
        if entree.codec_video:
            if entree.hauteur and entree.hauteur > hauteur_max:
                continue
            if entree.codec_video in disponibles:
                images.append(entree)
        elif entree.codec_audio in disponibles:
            sons.append(entree)

    if not images and not sons:
        return None, None
    images.sort(key=lambda f: (f.hauteur or 0), reverse=True)
    # Le son le plus leger suffit : la difference ne s'entend pas sur les
    # haut-parleurs d'une machine emulee, et chaque octet compte.
    sons.sort(key=lambda f: (f.octets or 0))
    return (images[0] if images else None), (sons[0] if sons else None)


def _decrit(format_):
    """Traduit une entree de `streamingData` en [`Flux`]."""
    type_mime = format_.get("mimeType", "")
    conteneur = ""
    if "/" in type_mime:
        conteneur = type_mime.split("/")[1].split(";")[0]

    codecs = ""
    if "codecs=" in type_mime:
        codecs = type_mime.split("codecs=")[1].strip('"\' ')

    codec_video = codec_audio = None
    for codec in [c.strip() for c in codecs.split(",") if c.strip()]:
        racine = codec.split(".")[0]
        traduit = {"avc1": "h264", "avc3": "h264", "vp9": "vp9", "vp09": "vp9",
                   "vp8": "vp8", "mp4v": "mpeg4", "av01": "av1",
                   "mp4a": "aac", "opus": "opus", "vorbis": "vorbis",
                   "ec-3": "eac3", "ac-3": "ac3"}.get(racine)
        if traduit in ("h264", "vp9", "vp8", "mpeg4", "av1"):
            codec_video = traduit
        elif traduit is not None:
            codec_audio = traduit

    url = format_.get("url")
    signee = None
    if not url:
        # Adresse signee : elle arrive sous forme de chaine de parametres.
        chiffre = format_.get("signatureCipher") or format_.get("cipher")
        if not chiffre:
            return None
        parametres = urllib.parse.parse_qs(chiffre)
        url = (parametres.get("url") or [""])[0]
        signee = {
            "s": (parametres.get("s") or [""])[0],
            "sp": (parametres.get("sp") or ["signature"])[0],
        }
        if not url or not signee["s"]:
            return None

    try:
        octets = int(format_.get("contentLength") or 0)
    except ValueError:
        octets = 0

    return Flux(
        url=url,
        itag=format_.get("itag"),
        conteneur=conteneur,
        codec_video=codec_video,
        codec_audio=codec_audio,
        hauteur=format_.get("height") or 0,
        octets=octets,
        progressif=bool(codec_video and codec_audio),
        signature_a_dechiffrer=signee,
    )


# --- Metadonnees --------------------------------------------------------------

def details(donnees):
    """Titre, auteur, duree, description — de quoi batir une page lisible."""
    brut = donnees.get("videoDetails") or {}
    try:
        secondes = int(brut.get("lengthSeconds") or 0)
    except ValueError:
        secondes = 0
    vignettes = ((brut.get("thumbnail") or {}).get("thumbnails") or [])
    return {
        "titre": brut.get("title") or "Video YouTube",
        "auteur": brut.get("author") or "",
        "duree": secondes,
        "description": brut.get("shortDescription") or "",
        "vues": brut.get("viewCount") or "",
        "vignette": vignettes[-1]["url"] if vignettes else "",
        "identifiant": brut.get("videoId") or "",
    }


def duree_lisible(secondes):
    if not secondes:
        return ""
    heures, reste = divmod(int(secondes), 3600)
    minutes, secondes = divmod(reste, 60)
    if heures:
        return "%d:%02d:%02d" % (heures, minutes, secondes)
    return "%d:%02d" % (minutes, secondes)
