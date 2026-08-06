"""Sonde media : decoder du H.264 et du son, sur la machine.

Lancee par
`exec /bo-navigateur /usr/share/bo-navigateur/media-probe.py`.

Elle fabrique elle-meme ce qu'elle decode. C'est volontaire : dependre d'un
fichier telecharge rendrait la sonde tributaire du reseau, et embarquer une
video toute faite ferait grossir le disque pour rien. Un WAV et un AVI se
construisent en quelques dizaines de lignes, sans encodeur, et suffisent a
prouver que la chaine tourne.

Ce qu'elle etablit :

- libavcodec est bien lie et rapporte ses decodeurs, H.264 et VP9 compris ;
- un flux reel est ouvert, ses pistes reconnues, ses trames decodees ;
- l'image sort en BGRA aux dimensions demandees ;
- le son sort en PCM 16 bits 48 kHz stereo, pret pour `/dev/dsp` ;
- l'alimentation par morceaux — le modele des Media Source Extensions —
  aboutit au meme resultat qu'un flux fourni d'un bloc.
"""

import struct
import sys

import bomedia

_echecs = []
_reussites = 0


def verifie(nom, condition, detail=""):
    global _reussites
    if condition:
        _reussites += 1
    else:
        _echecs.append("%s%s" % (nom, (" — " + str(detail)) if detail else ""))


def egal(nom, obtenu, attendu):
    verifie(nom, obtenu == attendu, "obtenu %r, attendu %r" % (obtenu, attendu))


# --- Fabrication d'un flux de test -------------------------------------------

def wav(frequence=44100, secondes=0.25, hauteur=440.0):
    """Un fichier WAV PCM 16 bits mono, entierement construit ici.

    Le WAV est le seul format audio qu'on puisse ecrire sans encodeur : il n'y a
    pas de compression, seulement un en-tete et des echantillons. C'est donc lui
    qui sert a eprouver la chaine de decodage audio de bout en bout.
    """
    trames = int(frequence * secondes)
    echantillons = bytearray()
    # Sinusoide par recurrence, sans bibliotheque mathematique.
    import math
    for i in range(trames):
        valeur = int(12000 * math.sin(2 * math.pi * hauteur * i / frequence))
        echantillons += struct.pack("<h", valeur)

    donnees = bytes(echantillons)
    return (b"RIFF" + struct.pack("<I", 36 + len(donnees)) + b"WAVE"
            + b"fmt " + struct.pack("<IHHIIHH", 16, 1, 1, frequence,
                                    frequence * 2, 2, 16)
            + b"data" + struct.pack("<I", len(donnees)) + donnees)


# --- Verifications ------------------------------------------------------------

def verifie_codecs():
    disponibles = set(bomedia.codecs())
    print("  FFmpeg %s, %d decodeurs" % (bomedia.version(), len(disponibles)))
    for nom in ("h264", "vp8", "vp9", "aac", "opus", "vorbis", "mp3", "mjpeg"):
        verifie("codec %s present" % nom, nom in disponibles)


def verifie_audio():
    """Une piste audio reelle, decodee et convertie au format de la carte."""
    source = bomedia.ouvre(wav())
    info = bomedia.info(source)
    verifie("wav: une piste audio est reconnue", info["a_audio"], info)
    verifie("wav: pas de piste video", not info["a_video"], info)
    verifie("wav: la duree est plausible",
            0.2 < info["duree"] < 0.3, info["duree"])

    total = 0
    trames = 0
    frequence = 0
    voies = 0
    while True:
        trame = bomedia.trame(source)
        if trame is None:
            break
        if trame["type"] != "audio":
            continue
        trames += 1
        total += len(trame["donnees"])
        frequence = trame["frequence"]
        voies = trame["voies"]

    verifie("wav: des trames audio sont sorties", trames > 0, trames)
    egal("wav: converti en 48 kHz", frequence, 48000)
    egal("wav: converti en stereo", voies, 2)
    # 0,25 s a 48 kHz, stereo, 16 bits = 48000 × 0,25 × 2 × 2 = 48000 octets.
    verifie("wav: la quantite de PCM correspond a la duree",
            40000 < total < 56000, total)
    print("  audio decode : %d octets de PCM en %d trames" % (total, trames))
    bomedia.ferme(source)


def verifie_alimentation():
    """Le modele des Media Source Extensions : le flux arrive par morceaux."""
    complet = wav(secondes=0.1)
    coupure = len(complet) // 2

    source = bomedia.ouvre(complet[:coupure])
    taille = bomedia.alimente(source, complet[coupure:])
    egal("mse: le tampon contient tout apres alimentation", taille, len(complet))

    total = 0
    while True:
        trame = bomedia.trame(source)
        if trame is None:
            break
        if trame["type"] == "audio":
            total += len(trame["donnees"])
    verifie("mse: le flux alimente en deux fois se decode", total > 10000, total)
    bomedia.ferme(source)


def verifie_video():
    """Une piste video reelle : trois JPEG dans un conteneur AVI.

    MJPEG plutot que H.264 parce qu'on peut fabriquer le flux ici, sans
    encodeur : chaque trame est une image JPEG independante, et l'AVI est le
    conteneur le plus simple a ecrire a la main. Le chemin exerce est le meme
    que pour H.264 — demultiplexage, decodeur, mise a l'echelle, conversion en
    BGRA ; seul le decodeur change, et sa presence est verifiee a part.
    """
    trames = [_jpeg_uni(32, 24, teinte) for teinte in (0, 1, 2)]

    flux = _avi_mjpeg(trames, 32, 24, 5)
    try:
        source = bomedia.ouvre(flux, 64, 48)
    except bomedia.Erreur as e:
        verifie("avi: le flux est lisible", False, e)
        return

    info = bomedia.info(source)
    verifie("avi: une piste video est reconnue", info["a_video"], info)
    egal("avi: le codec est mjpeg", info["codec_video"], "mjpeg")
    egal("avi: la largeur demandee est respectee", info["largeur"], 64)
    egal("avi: la hauteur demandee est respectee", info["hauteur"], 48)

    images = 0
    taille_attendue = 64 * 48 * 4
    while True:
        trame = bomedia.trame(source)
        if trame is None:
            break
        if trame["type"] != "video":
            continue
        images += 1
        egal("avi: l'image sort en BGRA a la bonne taille",
             len(trame["donnees"]), taille_attendue)
    verifie("avi: les trois trames sont decodees", images == 3, images)
    print("  video decodee : %d images de %dx%d" % (images, 64, 48))
    bomedia.ferme(source)


def _jpeg_uni(largeur, hauteur, teinte):
    """Une des trois images JPEG de reference."""
    del largeur, hauteur  # elles sont fixees par l'image embarquee
    return _JPEG_ESSAI[teinte % len(_JPEG_ESSAI)]


def _avi_mjpeg(trames, largeur, hauteur, images_par_seconde):
    """Assemble des JPEG en un AVI minimal.

    L'AVI est le conteneur le plus simple a ecrire a la main : deux listes
    imbriquees, un en-tete de flux, et les trames a la suite. Cela evite de
    dependre d'un encodeur pour eprouver le demultiplexeur.
    """
    def bloc(tag, charge):
        rembourrage = b"\x00" if len(charge) % 2 else b""
        return tag + struct.pack("<I", len(charge)) + charge + rembourrage

    def liste(tag, charge):
        return b"LIST" + struct.pack("<I", len(charge) + 4) + tag + charge

    duree_trame = 1000000 // images_par_seconde
    avih = struct.pack("<IIIIIIIIIIIIIIII",
                       duree_trame, 0, 0, 0x10, len(trames), 0, 1,
                       max(len(t) for t in trames), largeur, hauteur, 0, 0, 0, 0, 0, 0)
    strh = (b"vids" + b"MJPG" + struct.pack("<IHHIIIIIIIIi",
                                            0, 0, 0, 0, 1, images_par_seconde,
                                            0, len(trames),
                                            max(len(t) for t in trames), 0, 0, 0))
    strf = struct.pack("<IiiHHIIiiII", 40, largeur, hauteur, 1, 24,
                       0x47504A4D, largeur * hauteur * 3, 0, 0, 0, 0)

    entetes = liste(b"hdrl", bloc(b"avih", avih)
                    + liste(b"strl", bloc(b"strh", strh) + bloc(b"strf", strf)))
    corps = liste(b"movi", b"".join(bloc(b"00dc", t) for t in trames))
    return b"RIFF" + struct.pack("<I", len(entetes) + len(corps) + 4) + b"AVI " \
        + entetes + corps


# Trois JPEG 32x24 unis (rouge, vert, bleu), produits hors ligne. Ils sont en
# clair ici pour que la sonde ne depende d'aucun encodeur.
import base64  # noqa: E402

_JPEG_ESSAI = [base64.b64decode("".join(t)) for t in (
    # rouge uni, 32x24, qualite 80
    (
        "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAYEBQYFBAYGBQYHBwYIChAKCgkJChQODwwQFxQY"
        "GBcUFhYaHSUfGhsjHBYWICwgIyYnKSopGR8tMC0oMCUoKSj/2wBDAQcHBwoIChMKChMoGhYa"
        "KCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCj/wAAR"
        "CAAYACADASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAA"
        "AgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkK"
        "FhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWG"
        "h4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl"
        "5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREA"
        "AgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYk"
        "NOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOE"
        "hYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk"
        "5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDkqKKK+dP2cKKKKACiiigAooooA//Z"
    ),
    # vert uni, 32x24, qualite 80
    (
        "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAYEBQYFBAYGBQYHBwYIChAKCgkJChQODwwQFxQY"
        "GBcUFhYaHSUfGhsjHBYWICwgIyYnKSopGR8tMC0oMCUoKSj/2wBDAQcHBwoIChMKChMoGhYa"
        "KCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCj/wAAR"
        "CAAYACADASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAA"
        "AgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkK"
        "FhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWG"
        "h4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl"
        "5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREA"
        "AgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYk"
        "NOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOE"
        "hYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk"
        "5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDQooor58/LgooooAKKKKACiiigD//Z"
    ),
    # bleu uni, 32x24, qualite 80
    (
        "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAYEBQYFBAYGBQYHBwYIChAKCgkJChQODwwQFxQY"
        "GBcUFhYaHSUfGhsjHBYWICwgIyYnKSopGR8tMC0oMCUoKSj/2wBDAQcHBwoIChMKChMoGhYa"
        "KCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCj/wAAR"
        "CAAYACADASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAA"
        "AgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkK"
        "FhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWG"
        "h4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl"
        "5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREA"
        "AgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYk"
        "NOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOE"
        "hYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk"
        "5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDkKKKK/Uj5wKKKKACiiigAooooA//Z"
    ),
)]


def principal():
    print("=== media-probe ===")
    for verification in (verifie_codecs, verifie_audio, verifie_alimentation,
                         verifie_video):
        nom = verification.__name__.replace("verifie_", "")
        avant = len(_echecs)
        try:
            verification()
        except Exception as e:  # noqa: BLE001
            import traceback
            traceback.print_exc(file=sys.stdout)
            _echecs.append("%s a leve : %s" % (nom, e))
        print("  %s  %s" % ("ok " if len(_echecs) == avant else "ECHEC", nom))

    print()
    if _echecs:
        for echec in _echecs:
            print("  - %s" % echec)
        print("RESULTAT : %d verification(s) en echec sur %d"
              % (len(_echecs), len(_echecs) + _reussites))
        return 1
    print("RESULTAT : 0 verification(s) en echec (%d passees)" % _reussites)
    return 0


if __name__ == "__main__":
    sys.exit(principal())
