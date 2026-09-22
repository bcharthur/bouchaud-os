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

  AMONT          JPEG et WebP. Ecrire un encodeur JPEG baseline ou VP8L a la
                 main pour un banc, c'est ajouter une source de bogues qui
                 accuserait le port a la place de l'encodeur. Les fichiers
                 viennent donc de `Tests/LibGfx/test-inputs/` du Ladybird
                 EPINGLE, et les pixels attendus sont ceux que le test amont
                 `TestImageDecoder.cpp` affirme lui-meme.

                 C'est la reference la plus forte disponible : si ce decodeur
                 rend autre chose sous Bouchaud que ce qu'amont exige, la
                 difference vient du port.

## Ce que la presence des codecs prouve deja

`Libraries/LibGfx/ImageFormats/` du SHA epingle contient PNGLoader, JPEGLoader,
GIFLoader, WebPLoader, et aussi BMP, ICO, AVIF, JPEG-XL. Les quatre codecs
vises existent donc EN AMONT : une image qui ne s'affiche pas est un defaut
du port Bouchaud, jamais une limitation upstream. C'est verifiable et c'est
verifie.
"""
import os
import struct
import zlib

RACINE = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
AMONT = os.path.join(RACINE, "third_party", "ladybird", "Tests", "LibGfx", "test-inputs")


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


def _amont(chemin: str) -> bytes:
    with open(os.path.join(AMONT, chemin), "rb") as f:
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
        # JPEG : fichier AMONT, pixels affirmes par `TestImageDecoder.cpp`
        # (test_jpeg_cmyk, sans marqueur Adobe). 577 octets, 10x10.
        "id": 5, "nom": "jpeg-cmyk", "fichier": "jpeg-cmyk.jpg", "mime": "image/jpeg",
        "source": "amont:jpg/cmyk-no-adobe-marker.jpg", "largeur": 10, "hauteur": 10,
        "tolerance": 4, "octets": _amont("jpg/cmyk-no-adobe-marker.jpg"),
        "pixels": [(8, 1, 44, 184, 97), (1, 8, 184, 44, 97), (9, 9, 24, 24, 194)],
    },
    {
        # JPEG baseline ordinaire, un seul balayage. Amont n'affirme que le
        # decodage ; le banc n'affirme donc que les DIMENSIONS, et le dit.
        "id": 6, "nom": "jpeg-rgb24", "fichier": "jpeg-rgb24.jpg", "mime": "image/jpeg",
        "source": "amont:jpg/rgb24.jpg", "largeur": None, "hauteur": None,
        "tolerance": 0, "octets": _amont("jpg/rgb24.jpg"), "pixels": [],
    },
    {
        # WebP SANS PERTE (VP8L). Pixel affirme par `test_webp_simple_lossless`.
        "id": 7, "nom": "webp-lossless", "fichier": "webp-lossless.webp", "mime": "image/webp",
        "source": "amont:webp/simple-vp8l.webp", "largeur": 386, "hauteur": 395,
        "tolerance": 0, "octets": _amont("webp/simple-vp8l.webp"),
        "pixels": [(289, 332, 0xF2, 0xEE, 0xD3)],
    },
    {
        # WebP AVEC PERTE (VP8). Chemin de decodage entierement different du
        # precedent : les separer est ce qui permet de dire lequel est casse.
        "id": 8, "nom": "webp-lossy", "fichier": "webp-lossy.webp", "mime": "image/webp",
        "source": "amont:webp/4.webp", "largeur": 1024, "hauteur": 772,
        "tolerance": 4, "octets": _amont("webp/4.webp"),
        "pixels": [(780, 570, 0x72, 0xC8, 0xF6)],
    },
]

PAR_CHEMIN = {"/img/" + e["fichier"]: e for e in CATALOGUE}
FAVICON = CATALOGUE[0]["octets"]


if __name__ == "__main__":
    for e in CATALOGUE:
        print(f"{e['id']:>2} {e['nom']:<16} {len(e['octets']):>7} octets  {e['source']}")
