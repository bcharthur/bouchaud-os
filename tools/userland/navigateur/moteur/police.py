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

Le meme principe, en plus dense : un repertoire compact, tout le contenu des
tables concatene et compresse **en un seul flux brotli**, et — pour les polices
TrueType — une **transformation** des tables `glyf` et `loca` qui reecrit les
contours sous une forme plus compressible. Il faut donc, dans l'ordre :
decompresser, redecouper, et defaire la transformation.

La decompression vient de l'hote (`bo.debrotli`) : brotli n'est pas dans la
bibliotheque standard de Python, mais le navigateur le lie deja — freetype en
depend. Sur une machine de developpement, le module `brotli` fait l'affaire.

Une police en CFF (`OTTO`, ce qu'emploient la plupart des polices d'icones) n'a
pas de table `glyf` : la decompression et le redecoupage suffisent.
"""

import re
import struct
import zlib

# Les formats que l'on sait rendre a l'hote, du plus souhaitable au moins.
FORMATS_CONNUS = ("truetype", "opentype", "woff2", "woff")

_SIGNATURE_WOFF = b"wOFF"
_SIGNATURE_WOFF2 = b"wOF2"


def est_woff(octets):
    return octets[:4] == _SIGNATURE_WOFF


def est_woff2(octets):
    return octets[:4] == _SIGNATURE_WOFF2


def _debrotli(octets):
    """Decompresse un flux brotli. `None` si personne ne sait le faire ici.

    L'hote le sait : le navigateur lie deja `libbrotlidec`. Sur une machine de
    developpement, le module `brotli` prend le relais.
    """
    try:
        import bo
        fonction = getattr(bo, "debrotli", None)
        if fonction is not None:
            return fonction(bytes(octets))
    except ImportError:
        pass
    try:
        import brotli
        return brotli.decompress(bytes(octets))
    except Exception:  # noqa: BLE001
        return None


def ouvre(octets):
    """Rend une police `sfnt` (TrueType/OpenType) lisible par l'hote.

    `None` si le format n'est pas ouvrable — WOFF2, ou fichier abime.
    """
    if not octets:
        return None
    if est_woff2(octets):
        try:
            return _decompresse_woff2(octets)
        except (struct.error, ValueError, IndexError, KeyError):
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


# --- WOFF2 --------------------------------------------------------------------

# Les soixante-trois etiquettes que le format numerote, dans son ordre. Un
# repertoire WOFF2 ne cite une etiquette en toutes lettres que si elle n'est pas
# dans cette liste — c'est la moitie de ce qu'il economise.
_ETIQUETTES = [
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post",
    b"cvt ", b"fpgm", b"glyf", b"loca", b"prep", b"CFF ", b"VORG", b"EBDT",
    b"EBLC", b"gasp", b"hdmx", b"kern", b"LTSH", b"PCLT", b"VDMX", b"vhea",
    b"vmtx", b"BASE", b"GDEF", b"GPOS", b"GSUB", b"EBSC", b"JSTF", b"MATH",
    b"CBDT", b"CBLC", b"COLR", b"CPAL", b"SVG ", b"sbix", b"acnt", b"avar",
    b"bdat", b"bloc", b"bsln", b"cvar", b"fdsc", b"feat", b"fmtx", b"fvar",
    b"gvar", b"hsty", b"just", b"lcar", b"mort", b"morx", b"opbd", b"prop",
    b"trak", b"Zapf", b"Silf", b"Glat", b"Gloc", b"Feat", b"Sill",
]


class _Flux:
    """Un lecteur d'octets qui avance, avec les entiers du format WOFF2."""

    __slots__ = ("donnees", "position")

    def __init__(self, donnees, position=0):
        self.donnees = donnees
        self.position = position

    def prend(self, nombre):
        morceau = self.donnees[self.position:self.position + nombre]
        if len(morceau) != nombre:
            raise ValueError("flux WOFF2 tronque")
        self.position += nombre
        return morceau

    def octet(self):
        return self.prend(1)[0]

    def entier(self, format_):
        taille = struct.calcsize(format_)
        return struct.unpack(format_, self.prend(taille))[0]

    def base128(self):
        """`UIntBase128` : sept bits par octet, le huitieme dit s'il en reste."""
        valeur = 0
        for index in range(5):
            octet = self.octet()
            if index == 0 and octet == 0x80:
                raise ValueError("UIntBase128 a zero de tete")
            valeur = (valeur << 7) | (octet & 0x7F)
            if not octet & 0x80:
                return valeur
        raise ValueError("UIntBase128 trop long")

    def court255(self):
        """`255UInt16` : un encodage a trois cas, dense pour les petits nombres."""
        octet = self.octet()
        if octet == 253:
            return self.entier(">H")
        if octet == 255:
            return self.octet() + 253
        if octet == 254:
            return self.octet() + 253 * 2
        return octet

    def reste(self):
        return self.donnees[self.position:]


def _decompresse_woff2(octets):
    """Reconstitue le `sfnt` d'origine a partir du conteneur WOFF2."""
    if len(octets) < 48:
        raise ValueError("en-tete WOFF2 tronque")
    (saveur, _longueur, nombre, _reserve, _taille_sfnt,
     taille_comprimee) = struct.unpack(">4xIIHHII", octets[:24])

    flux = _Flux(octets, 48)
    entrees = []
    for _ in range(nombre):
        drapeaux = flux.octet()
        index = drapeaux & 0x3F
        transformation = (drapeaux >> 6) & 0x03
        etiquette = _ETIQUETTES[index] if index != 63 else flux.prend(4)
        origine = flux.base128()
        # La longueur transformee n'est ecrite que si la table l'est reellement.
        # `glyf` et `loca` sont transformees par defaut — c'est la version 3 qui
        # les laisse telles quelles ; toutes les autres, l'inverse.
        if etiquette in (b"glyf", b"loca"):
            transformee = transformation != 3
        else:
            transformee = transformation != 0
        taille = flux.base128() if transformee else origine
        entrees.append({"etiquette": etiquette, "origine": origine,
                        "taille": taille, "transformee": transformee})

    comprime = flux.prend(taille_comprimee)
    corps = _debrotli(comprime)
    if corps is None:
        raise ValueError("brotli indisponible")

    position = 0
    brutes = {}
    ordre = []
    for entree in entrees:
        brutes[entree["etiquette"]] = (corps[position:position + entree["taille"]],
                                       entree)
        ordre.append(entree["etiquette"])
        position += entree["taille"]

    tables = {}
    for etiquette in ordre:
        donnees, entree = brutes[etiquette]
        if etiquette == b"loca":
            continue          # reconstruite avec `glyf`
        if etiquette == b"glyf" and entree["transformee"]:
            glyf, loca = _defait_glyf(donnees)
            tables[b"glyf"] = glyf
            tables[b"loca"] = loca
            continue
        tables[etiquette] = donnees

    # `loca` non transformee doit etre reprise telle quelle.
    if b"loca" in brutes and b"loca" not in tables:
        donnees, entree = brutes[b"loca"]
        if not entree["transformee"]:
            tables[b"loca"] = donnees

    return _assemble_sfnt(saveur, sorted(tables.items()))


# --- La transformation des contours -------------------------------------------

def _defait_glyf(donnees):
    """Reconstruit `glyf` et `loca` a partir de leur forme transformee.

    Le format demonte chaque glyphe en sept flux paralleles — nombre de
    contours, nombre de points, drapeaux, coordonnees, composites, boites,
    instructions — parce que des valeurs de meme nature se compressent mieux
    ensemble. Les remonter, c'est parcourir les sept de front.
    """
    entete = _Flux(donnees)
    entete.prend(4)                       # reserve et drapeaux d'option
    nombre_glyphes = entete.entier(">H")
    format_index = entete.entier(">H")
    tailles = [entete.entier(">I") for _ in range(7)]

    debut = entete.position
    flux = {}
    for nom, taille in zip(("contours", "points", "drapeaux", "glyphes",
                            "composites", "boites", "instructions"), tailles):
        flux[nom] = _Flux(donnees, debut)
        debut += taille
    fin_boites = flux["boites"].position + tailles[5]

    # La carte des boites explicites : un bit par glyphe.
    octets_carte = (nombre_glyphes + 7) // 8
    carte = flux["boites"].prend(octets_carte)
    flux["boites"] = _Flux(donnees, flux["boites"].position)

    contenus = []
    for numero in range(nombre_glyphes):
        contours = flux["contours"].entier(">h")
        if contours == 0:
            contenus.append(b"")
            continue
        if contours < 0:
            contenus.append(_composite(flux, contours))
        else:
            contenus.append(_simple(flux, contours, numero, carte))

    # `loca` pointe sur chaque glyphe ; les glyphes s'alignent sur deux octets.
    corps = []
    decalages = [0]
    position = 0
    for contenu in contenus:
        bourrage = (-len(contenu)) % 4
        corps.append(contenu + b"\0" * bourrage)
        position += len(contenu) + bourrage
        decalages.append(position)

    if format_index == 0 and position <= 0x1FFFE:
        loca = b"".join(struct.pack(">H", d // 2) for d in decalages)
    else:
        loca = b"".join(struct.pack(">I", d) for d in decalages)
    _ = fin_boites
    return b"".join(corps), loca


def _simple(flux, contours, numero, carte):
    """Un glyphe ordinaire : ses contours, ses points, ses instructions."""
    fins = []
    total = 0
    for _ in range(contours):
        total += flux["points"].court255()
        fins.append(total - 1)

    drapeaux = []
    points = []
    x = y = 0
    for _ in range(total):
        drapeau = flux["drapeaux"].octet()
        sur_courbe = not (drapeau & 0x80)
        dx, dy = _triplet(flux["glyphes"], drapeau & 0x7F)
        x += dx
        y += dy
        drapeaux.append(1 if sur_courbe else 0)
        points.append((x, y))

    longueur_instructions = flux["glyphes"].court255()
    instructions = flux["instructions"].prend(longueur_instructions)

    # La boite : explicite si le bit du glyphe est leve, calculee sinon.
    if carte[numero >> 3] & (0x80 >> (numero & 7)):
        boite = struct.unpack(">hhhh", flux["boites"].prend(8))
    else:
        xs = [p[0] for p in points] or [0]
        ys = [p[1] for p in points] or [0]
        boite = (min(xs), min(ys), max(xs), max(ys))

    sortie = [struct.pack(">hhhhh", contours, *boite)]
    sortie.append(b"".join(struct.pack(">H", f) for f in fins))
    sortie.append(struct.pack(">H", longueur_instructions))
    sortie.append(instructions)
    sortie.append(_ecrit_points(drapeaux, points))
    return b"".join(sortie)


def _ecrit_points(sur_courbe, points):
    """Ecrit drapeaux et coordonnees dans la forme que `glyf` attend."""
    drapeaux = []
    xs = []
    ys = []
    precedent_x = precedent_y = 0
    for index, (x, y) in enumerate(points):
        drapeau = 0x01 if sur_courbe[index] else 0x00
        dx = x - precedent_x
        dy = y - precedent_y
        precedent_x, precedent_y = x, y

        if dx == 0:
            drapeau |= 0x10
        elif -255 <= dx <= 255:
            drapeau |= 0x02
            if dx > 0:
                drapeau |= 0x10
            xs.append(struct.pack(">B", abs(dx)))
        else:
            xs.append(struct.pack(">h", dx))

        if dy == 0:
            drapeau |= 0x20
        elif -255 <= dy <= 255:
            drapeau |= 0x04
            if dy > 0:
                drapeau |= 0x20
            ys.append(struct.pack(">B", abs(dy)))
        else:
            ys.append(struct.pack(">h", dy))
        drapeaux.append(struct.pack(">B", drapeau))
    return b"".join(drapeaux) + b"".join(xs) + b"".join(ys)


def _composite(flux, contours):
    """Un glyphe compose d'autres glyphes : on recopie sa description."""
    debut = flux["composites"].position
    plus = True
    instructions = False
    while plus:
        drapeaux = flux["composites"].entier(">H")
        flux["composites"].entier(">H")               # numero du composant
        plus = bool(drapeaux & 0x0020)                # MORE_COMPONENTS
        instructions = instructions or bool(drapeaux & 0x0100)
        flux["composites"].prend(4 if drapeaux & 0x0001 else 2)
        if drapeaux & 0x0008:
            flux["composites"].prend(2)
        elif drapeaux & 0x0040:
            flux["composites"].prend(4)
        elif drapeaux & 0x0080:
            flux["composites"].prend(8)
    corps = flux["composites"].donnees[debut:flux["composites"].position]

    boite = struct.unpack(">hhhh", flux["boites"].prend(8))
    sortie = [struct.pack(">hhhhh", contours, *boite), corps]
    if instructions:
        longueur = flux["glyphes"].court255()
        sortie.append(struct.pack(">H", longueur))
        sortie.append(flux["instructions"].prend(longueur))
    return b"".join(sortie)


def _signe(drapeau, valeur):
    """Le bit de poids faible du drapeau porte le signe : pose il est positif."""
    return valeur if drapeau & 1 else -valeur


def _triplet(flux, drapeau):
    """Decode un couple `(dx, dy)` du codage a triplets du format.

    Le codage range ensemble le nombre d'octets, les signes et les paliers : un
    point tres proche du precedent tient sur un octet, un point lointain sur
    quatre. C'est l'essentiel de ce que WOFF2 gagne sur les contours.

    Le drapeau porte deux signes a la fois — son bit 0 pour l'abscisse, son bit
    1 pour l'ordonnee — d'ou le decalage dans les appels a [`_signe`].
    """
    if drapeau < 84:
        octets = 1
    elif drapeau < 120:
        octets = 2
    elif drapeau < 124:
        octets = 3
    else:
        octets = 4
    donnees = flux.prend(octets)

    if drapeau < 10:
        # Deplacement vertical pur, sur onze bits.
        return 0, _signe(drapeau, ((drapeau & 14) << 7) + donnees[0])
    if drapeau < 20:
        # Deplacement horizontal pur.
        return _signe(drapeau, (((drapeau - 10) & 14) << 7) + donnees[0]), 0
    if drapeau < 84:
        # Les deux tiennent dans un octet : quatre bits chacun, plus un palier.
        reste = drapeau - 20
        octet = donnees[0]
        return (_signe(drapeau, 1 + (reste & 0x30) + (octet >> 4)),
                _signe(drapeau >> 1, 1 + ((reste & 0x0C) << 2) + (octet & 0x0F)))
    if drapeau < 120:
        reste = drapeau - 84
        return (_signe(drapeau, 1 + ((reste // 12) << 8) + donnees[0]),
                _signe(drapeau >> 1, 1 + (((reste % 12) >> 2) << 8) + donnees[1]))
    if drapeau < 124:
        # Douze bits chacun, a cheval sur l'octet du milieu.
        milieu = donnees[1]
        return (_signe(drapeau, (donnees[0] << 4) + (milieu >> 4)),
                _signe(drapeau >> 1, ((milieu & 0x0F) << 8) + donnees[2]))
    return (_signe(drapeau, (donnees[0] << 8) + donnees[1]),
            _signe(drapeau >> 1, (donnees[2] << 8) + donnees[3]))


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
