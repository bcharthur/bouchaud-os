#!/usr/bin/env python3
"""Le catalogue d'images du banc, et d'ou vient chaque octet.

## Pourquoi ce fichier existe

Le smoke test prouvait UNE chose : un PNG de 1x1 pixel se decode. Ce n'est
pas « les images fonctionnent ». Le defaut observe sur la machine physique
est justement celui qu'un PNG 1x1 ne peut pas montrer :

    la ressource arrive en HTTP 200, ImageDecoder tourne, et l'image reste
    BLANCHE a l'ecran.

Une image n'est donc OK que si ses PIXELS sont corrects apres decodage -- pas
si un appel a rendu « succes ».

## Deux provenances, et le choix est deliberé

  GENERE ICI     PNG et GIF. Ils sont produits par ce fichier, en zlib et en
                 LZW litteral, si bien que la couleur attendue de chaque
                 pixel est connue par CONSTRUCTION. Aucun encodeur tiers ne
                 s'interpose entre l'attente et le fichier.

  FIXTURES       JPEG et WebP. Les octets sont versionnes avec Bouchaud OS
  AUTONOMES      sous `tools/health/fixtures/browser-images/`, avec SHA-256.
                 Le banc Fast/Reliability ne depend donc plus d'un checkout
                 complet de Ladybird. Les quatre codecs restent testes PAR
                 LADYBIRD : la page charge les fichiers, dessine dans canvas
                 et relit des pixels connus.

                 BOUCHAUD_V13_FIXTURES_AUTONOMES

## Ce que la presence des codecs prouve deja

`Libraries/LibGfx/ImageFormats/` du SHA epingle contient PNGLoader, JPEGLoader,
GIFLoader, WebPLoader, et aussi BMP, ICO, AVIF, JPEG-XL. Les quatre codecs
vises existent donc EN AMONT : une image qui ne s'affiche pas est un defaut
du port Bouchaud, jamais une limitation upstream. C'est verifiable et c'est
verifie.
"""
import os
import sys
import struct
import zlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

RACINE = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FIXTURES = os.path.join(RACINE, "tools", "health", "fixtures", "browser-images")

# LES DEUX PROVENANCES COEXISTENT, ET C'EST VOULU.
#
# `fixtures/browser-images/` porte des images SYNTHETIQUES, produites ici et
# versionnees : leurs pixels sont connus par construction et elles couvrent
# chaque codec.
#
# `images_amont.py` porte les fichiers du CORPUS DE TEST du Ladybird epingle.
# Ils apportent ce que les synthetiques ne peuvent pas : `TestImageDecoder.cpp`
# affirme lui-meme certains de leurs pixels, si bien qu'un ecart accuse le
# port et rien d'autre. Ils couvrent aussi des chemins qu'une image produite
# proprement n'emprunte jamais -- un JPEG CMYK sans marqueur Adobe, un WebP
# a canal alpha.
#
# Les garder tous les deux elargit la couverture sans rien retirer.
import images_amont  # noqa: E402

AMONT = os.path.join(RACINE, "third_party", "ladybird", "Tests", "LibGfx", "test-inputs")

FIXTURE_SHA256 = {
    "jpeg-rgb-checker.jpg": "aa908d27599e54a73b8ee82866bcd2c2d9ef09f6862b5b11eaf60030e6064e84",
    "jpeg-rgb-gradient.jpg": "2d0f67f22353e7917075d870fa2d8be5953d2231b8fa8784fc64f99ecfdc85e0",
    "webp-lossless.webp": "5f8bd924c769c888fe0c98fe302633b871f73a5263fc6970c1ce3848aef7531a",
    "webp-lossy.webp": "bcb33bbe05b1141511427eaed5f46b552a00b0860905aecebdd08a038f0cf9ed",
}


# --------------------------------------------------------------------------
# PNG : genere, donc pixels connus par construction.
# --------------------------------------------------------------------------
def _png_chunk(kind: bytes, payload: bytes) -> bytes:
    body = kind + payload
    return struct.pack(">I", len(payload)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


def png_rgba(largeur: int, hauteur: int, pixel) -> bytes:
    """PNG RGBA sans entrelacement. `pixel(x, y)` rend (r, g, b, a)."""
    lignes = bytearray()
    for y in range(hauteur):
        lignes.append(0)  # filtre None : la ligne est ecrite telle quelle
        for x in range(largeur):
            lignes.extend(pixel(x, y))
    return (
        b"\x89PNG\r\n\x1a\n"
        + _png_chunk(b"IHDR", struct.pack(">IIBBBBB", largeur, hauteur, 8, 6, 0, 0, 0))
        + _png_chunk(b"IDAT", zlib.compress(bytes(lignes), 9))
        + _png_chunk(b"IEND", b"")
    )


# --------------------------------------------------------------------------
# GIF : genere, en LZW LITTERAL.
#
# Un encodeur LZW complet n'est pas necessaire et serait une source de bogues
# de plus. La norme autorise a n'emettre que des codes litteraux entrecoupes
# de codes CLEAR : le decodeur n'apprend alors aucune sequence, la largeur de
# code ne grandit jamais au-dela de neuf bits, et le fichier reste valide.
# --------------------------------------------------------------------------
class _BitsLSB:
    """GIF empile ses codes du bit de poids faible vers le fort."""

    def __init__(self):
        self.octets = bytearray()
        self.accu = 0
        self.taille = 0

    def ecris(self, valeur: int, bits: int):
        self.accu |= (valeur & ((1 << bits) - 1)) << self.taille
        self.taille += bits
        while self.taille >= 8:
            self.octets.append(self.accu & 0xFF)
            self.accu >>= 8
            self.taille -= 8

    def termine(self) -> bytes:
        if self.taille:
            self.octets.append(self.accu & 0xFF)
            self.accu = 0
            self.taille = 0
        return bytes(self.octets)


def gif_palette(largeur: int, hauteur: int, palette, index, boucles=None, delais=None) -> bytes:
    """GIF 8 bits par pixel. `index(x, y)` (ou une liste de trames) rend l'index palette."""
    trames = index if isinstance(index, list) else [index]
    table = bytearray()
    for couleur in palette:
        table.extend(couleur)
    table.extend(b"\x00" * (3 * (256 - len(palette))))

    sortie = bytearray(b"GIF89a")
    # Descripteur logique : table globale presente, 256 entrees.
    sortie += struct.pack("<HHBBB", largeur, hauteur, 0xF7, 0, 0)
    sortie += table

    if len(trames) > 1 or boucles is not None:
        # Extension NETSCAPE : le nombre de boucles d'une animation.
        n = 0 if boucles is None else boucles
        sortie += b"\x21\xFF\x0BNETSCAPE2.0\x03\x01" + struct.pack("<H", n) + b"\x00"

    for rang, trame in enumerate(trames):
        if len(trames) > 1:
            delai = 10 if delais is None else delais[rang]
            # Graphic Control : methode 2 (restaurer le fond) pour que chaque
            # trame remplace la precedente au lieu de s'y superposer.
            sortie += b"\x21\xF9\x04" + struct.pack("<BHB", 0x08, delai, 0) + b"\x00"
        sortie += b"\x2C" + struct.pack("<HHHHB", 0, 0, largeur, hauteur, 0)
        sortie += bytes([8])  # taille de code LZW minimale

        bits = _BitsLSB()
        CLEAR, FIN = 256, 257
        bits.ecris(CLEAR, 9)
        depuis_clear = 0
        for y in range(hauteur):
            for x in range(largeur):
                bits.ecris(trame(x, y), 9)
                depuis_clear += 1
                # Le dictionnaire du decodeur grandit d'une entree par code ;
                # on le vide avant qu'il n'impose dix bits.
                if depuis_clear >= 250:
                    bits.ecris(CLEAR, 9)
                    depuis_clear = 0
        bits.ecris(FIN, 9)
        donnees = bits.termine()

        for debut in range(0, len(donnees), 255):
            bloc = donnees[debut:debut + 255]
            sortie += bytes([len(bloc)]) + bloc
        sortie += b"\x00"

    sortie += b"\x3B"
    return bytes(sortie)


def _fixture(nom: str) -> bytes:
    """Lit une fixture versionnee avec le banc, jamais un checkout externe."""
    with open(os.path.join(FIXTURES, nom), "rb") as f:
        return f.read()


# --------------------------------------------------------------------------
# Le catalogue.
# --------------------------------------------------------------------------
# `pixels` : (x, y, r, g, b) attendus. `tolerance` : ecart admis par canal.
#
# La tolerance n'est pas une facilite : un JPEG est une compression AVEC
# PERTE, et la valeur exacte depend de l'implementation de l'IDCT. Amont
# affirme ces valeurs pour SON decodeur, c'est-a-dire celui que nous
# compilons -- la tolerance couvre le seul passage par le canvas.
_DAMIER = lambda x, y: (255, 0, 0, 255) if (x // 8 + y // 8) % 2 == 0 else (0, 0, 255, 255)

CATALOGUE = [
    {
        "id": 1, "nom": "png-damier", "fichier": "png-damier.png", "mime": "image/png",
        "source": "genere", "largeur": 32, "hauteur": 32, "tolerance": 0,
        "octets": png_rgba(32, 32, _DAMIER),
        "pixels": [(4, 4, 255, 0, 0), (12, 4, 0, 0, 255), (28, 28, 255, 0, 0)],
    },
    {
        "id": 2, "nom": "png-degrade", "fichier": "png-degrade.png", "mime": "image/png",
        "source": "genere", "largeur": 16, "hauteur": 16, "tolerance": 0,
        "octets": png_rgba(16, 16, lambda x, y: (x * 16, y * 16, 128, 255)),
        "pixels": [(0, 0, 0, 0, 128), (15, 15, 240, 240, 128), (8, 4, 128, 64, 128)],
    },
    {
        "id": 3, "nom": "gif-damier", "fichier": "gif-damier.gif", "mime": "image/gif",
        "source": "genere", "largeur": 32, "hauteur": 32, "tolerance": 0,
        "octets": gif_palette(
            32, 32,
            [(255, 0, 0), (0, 0, 255), (0, 255, 0)],
            lambda x, y: 0 if (x // 8 + y // 8) % 2 == 0 else 1,
        ),
        "pixels": [(4, 4, 255, 0, 0), (12, 4, 0, 0, 255), (28, 28, 255, 0, 0)],
    },
    {
        # GIF ANIME : deux trames, rouge puis vert. Le banc ne verifie que la
        # PREMIERE -- l'instant ou la seconde arrive depend de l'horloge
        # d'animation, et en faire une assertion rendrait le banc instable.
        # Ce qui est verifie ici est que l'animation ne casse pas le decodage.
        "id": 4, "nom": "gif-anime", "fichier": "gif-anime.gif", "mime": "image/gif",
        "source": "genere", "largeur": 16, "hauteur": 16, "tolerance": 0, "anime": True,
        "octets": gif_palette(
            16, 16,
            [(255, 0, 0), (0, 255, 0)],
            [lambda x, y: 0, lambda x, y: 1],
            boucles=0, delais=[50, 50],
        ),
        "pixels": [(8, 8, 255, 0, 0)],
    },
    {
        # JPEG baseline 4:4:4 autonome : quatre quadrants non uniformes.
        "id": 5, "nom": "jpeg-rgb-checker", "fichier": "jpeg-rgb-checker.jpg", "mime": "image/jpeg",
        "source": "fixture:jpeg-rgb-checker.jpg", "largeur": 32, "hauteur": 32,
        "tolerance": 10, "octets": _fixture("jpeg-rgb-checker.jpg"),
        "pixels": [(4, 4, 235, 35, 45), (24, 4, 27, 93, 233),
                   (4, 24, 35, 210, 81), (24, 24, 243, 202, 32)],
    },
    {
        # Deuxieme JPEG autonome : degrade RGB, pixels verifies eux aussi.
        "id": 6, "nom": "jpeg-rgb-gradient", "fichier": "jpeg-rgb-gradient.jpg", "mime": "image/jpeg",
        "source": "fixture:jpeg-rgb-gradient.jpg", "largeur": 32, "hauteur": 32,
        "tolerance": 10, "octets": _fixture("jpeg-rgb-gradient.jpg"),
        "pixels": [(4, 4, 32, 32, 32), (24, 4, 193, 33, 133),
                   (4, 24, 31, 191, 91), (24, 24, 192, 192, 192)],
    },
    {
        # WebP SANS PERTE (VP8L), fixture autonome.
        "id": 7, "nom": "webp-lossless", "fichier": "webp-lossless.webp", "mime": "image/webp",
        "source": "fixture:webp-lossless.webp", "largeur": 32, "hauteur": 32,
        "tolerance": 0, "octets": _fixture("webp-lossless.webp"),
        "pixels": [(4, 4, 236, 36, 46), (24, 4, 28, 93, 233),
                   (4, 24, 35, 210, 80), (24, 24, 242, 202, 32)],
    },
    {
        # WebP AVEC PERTE (VP8), fixture autonome.
        "id": 8, "nom": "webp-lossy", "fichier": "webp-lossy.webp", "mime": "image/webp",
        "source": "fixture:webp-lossy.webp", "largeur": 32, "hauteur": 32,
        "tolerance": 12, "octets": _fixture("webp-lossy.webp"),
        "pixels": [(4, 4, 240, 40, 51), (24, 4, 27, 94, 233),
                   (4, 24, 36, 210, 81), (24, 24, 240, 199, 29)],
    },
    {
        # JPEG CMYK SANS MARQUEUR ADOBE -- un chemin qu'aucune image produite
        # proprement n'emprunte. Les trois pixels sont ceux qu'affirme
        # `TestImageDecoder.cpp` (test_jpeg_cmyk). 577 octets.
        "id": 9, "nom": "jpeg-cmyk", "fichier": "jpeg-cmyk.jpg", "mime": "image/jpeg",
        "source": "amont:jpg/cmyk-no-adobe-marker.jpg", "largeur": 10, "hauteur": 10,
        "tolerance": 4, "octets": images_amont.octets("jpeg_cmyk"),
        "pixels": [(8, 1, 44, 184, 97), (1, 8, 184, 44, 97), (9, 9, 24, 24, 194)],
    },
    {
        # WebP AVEC PERTE (VP8), 784 octets, et ses deux pixels sont affirmes
        # par `test_webp_simple_lossy`.
        "id": 10, "nom": "webp-vp8-amont", "fichier": "webp-vp8-amont.webp",
        "mime": "image/webp", "source": "amont:webp/simple-vp8.webp",
        "largeur": 240, "hauteur": 240, "tolerance": 4,
        "octets": images_amont.octets("webp_vp8"),
        "pixels": [(120, 232, 0xF1, 0xEF, 0xF0), (198, 202, 0x7A, 0xAA, 0xD5)],
    },
    {
        # WebP A CANAL ALPHA, 38 octets. L'alpha est un etage separe du reste
        # du decodage : les huit images opaques ci-dessus ne disent rien de lui.
        #
        # L'assertion porte sur l'ALPHA et non sur la couleur, parce que le
        # canvas PREMULTIPLIE : un pixel blanc a moitie transparent n'en
        # ressort pas blanc, et une assertion de couleur echouerait sur une
        # image pourtant bien decodee.
        "id": 11, "nom": "webp-alpha", "fichier": "webp-alpha.webp", "mime": "image/webp",
        "source": "amont:webp/semi-transparent-pixel.webp", "largeur": 1, "hauteur": 1,
        "tolerance": 0, "octets": images_amont.octets("webp_alpha"),
        "pixels": [], "alpha_attendu": (0, 0, 128),
    },
]

PAR_CHEMIN = {"/img/" + e["fichier"]: e for e in CATALOGUE}
FAVICON = CATALOGUE[0]["octets"]


if __name__ == "__main__":
    for e in CATALOGUE:
        print(f"{e['id']:>2} {e['nom']:<16} {len(e['octets']):>7} octets  {e['source']}")
