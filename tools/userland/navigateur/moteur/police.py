"""Polices du Web : d'un fichier `@font-face` a une police que l'hote sait lire.

Un site ne livre presque jamais ses polices en TrueType : il les emballe en
**WOFF**, et de plus en plus en **WOFF2**. Ni Qt ni Pillow ne lisent ces deux
formats — ce sont des conteneurs, pas des polices. Il faut donc les ouvrir.

## WOFF

Un simple sac : un en-tete, un repertoire de tables, et chaque table rangee
telle quelle ou compressee par zlib. La rouvrir revient a decompresser chaque
table et a reconstituer le fichier `sfnt` que TrueType attend. Une centaine de
lignes, et rien d'autre que `zlib` — que tout Python porte, y compris celui de
l'OS.

## WOFF2

Compresse par brotli, avec en plus une **transformation** des tables `glyf` et
`loca` qu'il faut defaire. C'est un autre ordre de grandeur, et brotli n'est
pas dans la bibliotheque standard. Le format est donc reconnu et refuse
explicitement : mieux vaut une police manquante et un message qu'un fichier
illisible passe a l'hote.

En pratique, la plupart des sites declarent leurs sources dans l'ordre
`woff2`, `woff`, `truetype` : il suffit de prendre la premiere que l'on sait
ouvrir, ce que fait [`meilleure_source`].
"""

import re
import struct
import zlib

# Les formats que l'on sait rendre a l'hote, du plus souhaitable au moins.
FORMATS_CONNUS = ("truetype", "opentype", "woff")

_SIGNATURE_WOFF = b"wOFF"
_SIGNATURE_WOFF2 = b"wOF2"


def est_woff(octets):
    return octets[:4] == _SIGNATURE_WOFF


def est_woff2(octets):
    return octets[:4] == _SIGNATURE_WOFF2


def ouvre(octets):
    """Rend une police `sfnt` (TrueType/OpenType) lisible par l'hote.

    `None` si le format n'est pas ouvrable — WOFF2, ou fichier abime.
    """
    if not octets:
        return None
    if est_woff2(octets):
        return None
    if not est_woff(octets):
        # Deja du TrueType ou de l'OpenType : rien a faire.
        return octets if octets[:4] in (b"\x00\x01\x00\x00", b"true", b"ttcf",
                                        b"OTTO") else None
    try:
        return _decompresse_woff(octets)
    except (struct.error, zlib.error, ValueError, IndexError):
        return None


def _decompresse_woff(octets):
    """Reconstitue le `sfnt` d'origine a partir du conteneur WOFF."""
    if len(octets) < 44:
        raise ValueError("en-tete WOFF tronque")
    saveur, _longueur, nombre = struct.unpack(">4xIIH", octets[:14])

    entrees = []
    position = 44
    for _ in range(nombre):
        if position + 20 > len(octets):
            raise ValueError("repertoire WOFF tronque")
        tag, decalage, compresse, original, _somme = struct.unpack(
            ">4sIIII", octets[position:position + 20])
        entrees.append((tag, decalage, compresse, original))
        position += 20

    # Les tables se rangent dans l'ordre de leur etiquette, comme l'exige le
    # format sfnt ; le repertoire WOFF, lui, peut etre dans n'importe quel ordre.
    entrees.sort(key=lambda e: e[0])

    tables = []
    for tag, decalage, compresse, original in entrees:
        brut = octets[decalage:decalage + compresse]
        if compresse < original:
            corps = zlib.decompress(brut)
        else:
            corps = brut
        if len(corps) != original:
            raise ValueError("table %r de taille inattendue" % tag)
        tables.append((tag, corps))

    return _assemble_sfnt(saveur, tables)


def _assemble_sfnt(saveur, tables):
    """Ecrit un fichier sfnt : en-tete, repertoire, puis les tables alignees."""
    nombre = len(tables)
    # `searchRange`, `entrySelector` et `rangeShift` sont redondants avec le
    # nombre de tables ; certains lecteurs les verifient tout de meme.
    exposant = max(nombre.bit_length() - 1, 0)
    puissance = 1 << exposant
    entete = struct.pack(">IHHHH", saveur, nombre, puissance * 16, exposant,
                         nombre * 16 - puissance * 16)

    debut = 12 + nombre * 16
    repertoire = []
    corps = []
    decalage = debut
    for tag, contenu in tables:
        repertoire.append(struct.pack(">4sIII", tag, _somme(contenu), decalage,
                                      len(contenu)))
        corps.append(contenu)
        bourrage = (-len(contenu)) % 4
        if bourrage:
            corps.append(b"\0" * bourrage)
        decalage += len(contenu) + bourrage

    return entete + b"".join(repertoire) + b"".join(corps)


def _somme(contenu):
    """Somme de controle d'une table, comme la definit le format sfnt."""
    rembourre = contenu + b"\0" * ((-len(contenu)) % 4)
    total = 0
    for (mot,) in struct.iter_unpack(">I", rembourre):
        total = (total + mot) & 0xFFFFFFFF
    return total


# --- Analyse des declarations -------------------------------------------------

_SOURCE = re.compile(r"""url\(\s*(['"]?)(?P<url>[^'")]+)\1\s*\)
                         (?:\s*format\(\s*['"]?(?P<format>[^'")]+)['"]?\s*\))?""",
                     re.X | re.I)


def sources(declaration):
    """Les sources d'un `src:`, en couples `(url, format)`, dans l'ordre cite."""
    trouvees = []
    for m in _SOURCE.finditer(declaration or ""):
        format_ = (m.group("format") or "").strip().lower()
        if not format_:
            # Sans `format()`, l'extension le dit presque toujours.
            bas = m.group("url").lower().split("?")[0]
            for extension, nom in ((".woff2", "woff2"), (".woff", "woff"),
                                   (".ttf", "truetype"), (".otf", "opentype")):
                if bas.endswith(extension):
                    format_ = nom
                    break
        trouvees.append((m.group("url").strip(), format_))
    return trouvees


def meilleure_source(declaration):
    """La premiere source que l'on sait ouvrir. `None` s'il n'y en a aucune.

    L'ordre du site place le WOFF2 en tete, parce que c'est le plus compact ;
    on prend le suivant, en preferant toujours ce qui demande le moins de
    travail — une police deja en TrueType passe telle quelle.
    """
    trouvees = sources(declaration)
    for souhaite in FORMATS_CONNUS:
        for url, format_ in trouvees:
            if format_ == souhaite:
                return url, format_
    # Format inconnu ou absent : on tente, `ouvre` refusera si c'est illisible.
    for url, format_ in trouvees:
        if format_ != "woff2":
            return url, format_
    return None


# --- Plages Unicode -----------------------------------------------------------

# Le latin de base : c'est lui qu'une page occidentale dessine, et c'est donc
# la coupe qu'il faut retenir quand un site en livre une par ecriture.
LATIN = (0x20, 0x7E)

_PLAGE = re.compile(r"[uU]\+([0-9a-fA-F?]{1,6})(?:-([0-9a-fA-F]{1,6}))?")


def plages(declaration):
    """Les intervalles d'un `unicode-range`. Vide si la propriete est absente."""
    intervalles = []
    for debut, fin in _PLAGE.findall(declaration or ""):
        if "?" in debut:
            bas = int(debut.replace("?", "0"), 16)
            haut = int(debut.replace("?", "F"), 16)
        else:
            bas = int(debut, 16)
            haut = int(fin, 16) if fin else bas
        intervalles.append((bas, haut))
    return intervalles


def couvre_latin(declaration):
    """Cette coupe sert-elle a ecrire du latin ?

    Un site livre souvent une police par ecriture — cyrillique, grec, latin —
    sous **la meme famille**, chacune avec son `unicode-range`. Les charger
    toutes sous le meme nom fait gagner la derniere, et une page latine se
    retrouve ecrite dans une coupe qui n'en porte pas les glyphes : tout sort
    en carres. Faute de choisir la coupe caractere par caractere, on ne retient
    que celle qui sait ecrire du latin.
    """
    intervalles = plages(declaration)
    if not intervalles:
        return True
    return any(bas <= LATIN[1] and haut >= LATIN[0] for bas, haut in intervalles)


# --- Registre des familles chargees -------------------------------------------
#
# L'hote garde les polices ; le moteur retient seulement leurs noms, pour savoir
# quelle famille d'un `font-family` il peut reellement demander.
ENREGISTREES = set()


def oublie():
    ENREGISTREES.clear()


def retiens(famille):
    if famille:
        ENREGISTREES.add(famille.strip().lower())


def connue(famille):
    return bool(famille) and famille.strip().lower() in ENREGISTREES
