#!/usr/bin/env python3
"""Le banc d'images doit etre juste AVANT de servir a accuser le port.

Un encodeur qui se trompe produit exactement le meme symptome qu'un decodeur
casse : des pixels qui ne correspondent pas a l'attente. La difference est
qu'on passerait alors des heures a chercher dans Ladybird un defaut qui est
dans ce depot.

Ce fichier decode les images GENEREES avec un decodeur ecrit ici, independant
de l'encodeur, et verifie que les pixels annonces au banc sont bien ceux que
le fichier contient. Il verifie aussi que les fichiers AMONT sont presents et
portent leur signature -- leurs pixels, eux, sont affirmes par le test amont
et n'ont pas a etre redecodes ici.

    python3 tools/health/test_images_fixtures.py
"""
import os
import hashlib
import sys
import zlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import images_fixtures as fx  # noqa: E402

ECHECS = []


def verifie(condition, message):
    if not condition:
        ECHECS.append(message)


# --------------------------------------------------------------------------
# Un decodeur PNG minimal : assez pour le sous-ensemble que l'on produit.
# --------------------------------------------------------------------------
def decode_png(octets):
    verifie(octets[:8] == b"\x89PNG\r\n\x1a\n", "png : signature absente")
    pos, idat, entete = 8, bytearray(), None
    while pos < len(octets):
        taille = int.from_bytes(octets[pos:pos + 4], "big")
        genre = octets[pos + 4:pos + 8]
        charge = octets[pos + 8:pos + 8 + taille]
        attendu = zlib.crc32(genre + charge) & 0xFFFFFFFF
        recu = int.from_bytes(octets[pos + 8 + taille:pos + 12 + taille], "big")
        verifie(attendu == recu, f"png : CRC faux sur {genre!r}")
        if genre == b"IHDR":
            entete = (
                int.from_bytes(charge[0:4], "big"),
                int.from_bytes(charge[4:8], "big"),
                charge[8], charge[9],
            )
        elif genre == b"IDAT":
            idat += charge
        pos += 12 + taille
    largeur, hauteur, profondeur, couleur = entete
    verifie((profondeur, couleur) == (8, 6), "png : le banc ne produit que du RGBA 8 bits")
    brut = zlib.decompress(bytes(idat))
    par_ligne = largeur * 4
    pixels, precedente = [], bytearray(par_ligne)
    pos = 0
    for _ in range(hauteur):
        filtre = brut[pos]
        verifie(filtre == 0, f"png : filtre {filtre} inattendu (le banc n'ecrit que None)")
        ligne = bytearray(brut[pos + 1:pos + 1 + par_ligne])
        pixels.append(ligne)
        precedente = ligne
        pos += 1 + par_ligne
    return largeur, hauteur, pixels


# --------------------------------------------------------------------------
# Un decodeur GIF minimal, LZW COMPLET.
#
# Complet et non « litteral » : l'encodeur n'emet que des litteraux, mais un
# decodeur qui ne saurait lire que cela ne prouverait rien -- il accepterait
# un flux que les vrais decodeurs refusent.
# --------------------------------------------------------------------------
def decode_gif(octets):
    verifie(octets[:6] in (b"GIF87a", b"GIF89a"), "gif : signature absente")
    largeur = int.from_bytes(octets[6:8], "little")
    hauteur = int.from_bytes(octets[8:10], "little")
    drapeaux = octets[10]
    pos = 13
    palette = []
    if drapeaux & 0x80:
        n = 2 << (drapeaux & 0x07)
        for i in range(n):
            palette.append(tuple(octets[pos + 3 * i:pos + 3 * i + 3]))
        pos += 3 * n

    trames = []
    while pos < len(octets):
        bloc = octets[pos]
        if bloc == 0x3B:
            break
        if bloc == 0x21:  # extension
            pos += 2
            while octets[pos]:
                pos += 1 + octets[pos]
            pos += 1
            continue
        if bloc != 0x2C:
            ECHECS.append(f"gif : bloc inconnu {bloc:#x}")
            break
        iw = int.from_bytes(octets[pos + 5:pos + 7], "little")
        ih = int.from_bytes(octets[pos + 7:pos + 9], "little")
        local = octets[pos + 9]
        pos += 10
        verifie(not (local & 0x80), "gif : le banc n'ecrit pas de table locale")

        code_min = octets[pos]
        pos += 1
        donnees = bytearray()
        while octets[pos]:
            taille = octets[pos]
            donnees += octets[pos + 1:pos + 1 + taille]
            pos += 1 + taille
        pos += 1

        sortie = _lzw_gif(bytes(donnees), code_min)
        verifie(len(sortie) >= iw * ih, f"gif : {len(sortie)} pixels pour {iw}x{ih}")
        trames.append((iw, ih, sortie[:iw * ih]))
    return largeur, hauteur, palette, trames


def _lzw_gif(donnees, code_min):
    CLEAR, FIN = 1 << code_min, (1 << code_min) + 1
    dico, largeur_code, suivant = None, 0, 0

    def remets_a_zero():
        nonlocal dico, largeur_code, suivant
        dico = {i: bytes([i]) for i in range(1 << code_min)}
        largeur_code = code_min + 1
        suivant = FIN + 1

    remets_a_zero()
    sortie, precedent = bytearray(), None
    accu, bits, pos = 0, 0, 0
    while True:
        while bits < largeur_code:
            if pos >= len(donnees):
                return bytes(sortie)
            accu |= donnees[pos] << bits
            bits += 8
            pos += 1
        code = accu & ((1 << largeur_code) - 1)
        accu >>= largeur_code
        bits -= largeur_code

        if code == CLEAR:
            remets_a_zero()
            precedent = None
            continue
        if code == FIN:
            return bytes(sortie)
        if code in dico:
            entree = dico[code]
        elif code == suivant and precedent is not None:
            entree = precedent + precedent[:1]
        else:
            ECHECS.append(f"gif : code {code} hors dictionnaire")
            return bytes(sortie)
        sortie += entree
        if precedent is not None and suivant < 4096:
            dico[suivant] = precedent + entree[:1]
            suivant += 1
            if suivant == (1 << largeur_code) and largeur_code < 12:
                largeur_code += 1
        precedent = entree


# --------------------------------------------------------------------------
def controle():
    for entree in fx.CATALOGUE:
        nom, source = entree["nom"], entree["source"]

        # BOUCHAUD_V13_FIXTURES_AUTONOMES : Fast/Reliability ne doivent
        # jamais dependre d'un checkout Ladybird absent de leur job.
        if source.startswith("fixture:"):
            fichier = source.split(":", 1)[1]
            chemin = os.path.join(fx.FIXTURES, fichier)
            verifie(os.path.exists(chemin), f"{nom} : fixture absente -- {chemin}")
            octets = entree["octets"]
            verifie(len(octets) > 0, f"{nom} : fixture vide")
            if os.path.exists(chemin):
                disque = open(chemin, "rb").read()
                verifie(octets == disque, f"{nom} : octets catalogue != fixture")
                attendu = fx.FIXTURE_SHA256.get(fichier)
                verifie(attendu is not None, f"{nom} : SHA-256 non declare")
                if attendu is not None:
                    verifie(hashlib.sha256(disque).hexdigest() == attendu,
                            f"{nom} : SHA-256 fixture inattendu")
            if entree["mime"] == "image/jpeg":
                verifie(octets[:2] == b"\xff\xd8", f"{nom} : ce n'est pas un JPEG")
            elif entree["mime"] == "image/webp":
                verifie(octets[:4] == b"RIFF" and octets[8:12] == b"WEBP",
                        f"{nom} : ce n'est pas un WebP")
            continue

        if entree["mime"] == "image/png":
            largeur, hauteur, lignes = decode_png(entree["octets"])
            verifie(largeur == entree["largeur"] and hauteur == entree["hauteur"],
                    f"{nom} : {largeur}x{hauteur} au lieu de "
                    f"{entree['largeur']}x{entree['hauteur']}")
            for (x, y, r, g, b) in entree["pixels"]:
                lu = tuple(lignes[y][4 * x:4 * x + 3])
                verifie(lu == (r, g, b),
                        f"{nom} : pixel ({x},{y}) vaut {lu}, le banc annonce {(r, g, b)}")

        elif entree["mime"] == "image/gif":
            largeur, hauteur, palette, trames = decode_gif(entree["octets"])
            verifie(largeur == entree["largeur"] and hauteur == entree["hauteur"],
                    f"{nom} : {largeur}x{hauteur} au lieu de "
                    f"{entree['largeur']}x{entree['hauteur']}")
            attendu = 2 if entree.get("anime") else 1
            verifie(len(trames) == attendu,
                    f"{nom} : {len(trames)} trame(s), {attendu} attendue(s)")
            if trames:
                iw, _, indices = trames[0]
                for (x, y, r, g, b) in entree["pixels"]:
                    lu = palette[indices[y * iw + x]]
                    verifie(lu == (r, g, b),
                            f"{nom} : pixel ({x},{y}) vaut {lu}, le banc annonce {(r, g, b)}")

    # LE BANC DOIT COUVRIR LES QUATRE CODECS VISES.
    #
    # Sans cette regle, retirer une entree du catalogue rendrait le banc plus
    # vert sans rendre le navigateur meilleur.
    mimes = {e["mime"] for e in fx.CATALOGUE}
    for exige in ("image/png", "image/jpeg", "image/gif", "image/webp"):
        verifie(exige in mimes, f"catalogue : plus aucune image {exige}")
    verifie(any(e.get("anime") for e in fx.CATALOGUE), "catalogue : plus de GIF anime")


controle()
if ECHECS:
    print("images : le banc lui-meme est faux")
    for e in ECHECS:
        print("   ", e)
    raise SystemExit(1)
print(f"images : catalogue juste ({len(fx.CATALOGUE)} entrees, "
      f"{sum(len(e['octets']) for e in fx.CATALOGUE)} octets)")
