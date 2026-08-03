"""Metriques de texte et decoupe en lignes.

Le layout doit connaitre la largeur d'un mot avant de savoir s'il tient sur
la ligne courante. Les backends n'ont pas tous acces aux memes polices : on
fournit donc une metrique par defaut — proportionnelle, calibree sur une
grotesque type Helvetica — que chaque backend peut remplacer par la mesure
reelle de sa police.
"""

# Largeurs d'avance pour une police de 1000 unites par cadratin. Les valeurs
# viennent des metriques Helvetica ; elles approchent de tres pres Arial,
# Liberation Sans et les polices systeme utilisees par les backends.
_ADVANCES = {
    " ": 278, "!": 278, '"': 355, "#": 556, "$": 556, "%": 889, "&": 667,
    "'": 191, "(": 333, ")": 333, "*": 389, "+": 584, ",": 278, "-": 333,
    ".": 278, "/": 278,
    "0": 556, "1": 556, "2": 556, "3": 556, "4": 556, "5": 556, "6": 556,
    "7": 556, "8": 556, "9": 556,
    ":": 278, ";": 278, "<": 584, "=": 584, ">": 584, "?": 556, "@": 1015,
    "A": 667, "B": 667, "C": 722, "D": 722, "E": 667, "F": 611, "G": 778,
    "H": 722, "I": 278, "J": 500, "K": 667, "L": 556, "M": 833, "N": 722,
    "O": 778, "P": 667, "Q": 778, "R": 722, "S": 667, "T": 611, "U": 722,
    "V": 667, "W": 944, "X": 667, "Y": 667, "Z": 611,
    "[": 278, "\\": 278, "]": 278, "^": 469, "_": 556, "`": 333,
    "a": 556, "b": 556, "c": 500, "d": 556, "e": 556, "f": 278, "g": 556,
    "h": 556, "i": 222, "j": 222, "k": 500, "l": 222, "m": 833, "n": 556,
    "o": 556, "p": 556, "q": 556, "r": 333, "s": 500, "t": 278, "u": 556,
    "v": 500, "w": 722, "x": 500, "y": 500, "z": 500,
    "{": 334, "|": 260, "}": 334, "~": 584,
}

# Avance par defaut pour un caractere hors table (accents, symboles, CJK).
_DEFAULT_ADVANCE = 556
_CJK_ADVANCE = 1000

# Le gras elargit legerement les glyphes.
_BOLD_FACTOR = 1.06

# Une police monospace avance de la meme valeur pour tout caractere.
_MONO_ADVANCE = 600


class Metrics:
    """Mesure des chaines pour une famille de police donnee."""

    __slots__ = ("monospace",)

    def __init__(self, monospace=False):
        self.monospace = monospace

    def char_width(self, char, size, bold=False):
        return self.measure(char, size, bold)

    def measure(self, text, size, bold=False):
        """Largeur en pixels de `text` rendu a `size` pixels."""
        if not text:
            return 0
        if self.monospace:
            total = _MONO_ADVANCE * len(text)
        else:
            total = 0
            for char in text:
                code = ord(char)
                if code < 128:
                    total += _ADVANCES.get(char, _DEFAULT_ADVANCE)
                elif 0x4E00 <= code <= 0x9FFF or 0x3040 <= code <= 0x30FF:
                    # Ideogrammes et kana : chasse pleine.
                    total += _CJK_ADVANCE
                else:
                    total += _DEFAULT_ADVANCE
        width = total * size / 1000.0
        if bold:
            width *= _BOLD_FACTOR
        return int(round(width))

    def line_height(self, size):
        return int(round(size * 1.2))

    def ascent(self, size):
        return int(round(size * 0.8))


DEFAULT = Metrics()
MONOSPACE = Metrics(monospace=True)


def metrics_for(style):
    """Metrique correspondant a un `ComputedStyle`."""
    return MONOSPACE if style.monospace else DEFAULT


def normalize_whitespace(text):
    """Effondre les blancs comme le fait `white-space: normal`."""
    if not text:
        return ""
    out = []
    space = False
    for char in text:
        if char in " \t\n\r\f\v":
            space = True
            continue
        if space and out:
            out.append(" ")
        space = False
        out.append(char)
    result = "".join(out)
    # On preserve l'information « il y avait un blanc au bord » : c'est ce qui
    # empeche `<b>a</b> <b>b</b>` de se coller en `ab`.
    if text[0] in " \t\n\r\f\v":
        result = " " + result
    if text[-1] in " \t\n\r\f\v" and len(text) > 1:
        result = result + " "
    return result


def split_words(text):
    """Decoupe en unites de rupture, en gardant les espaces significatifs.

    Renvoie une liste de morceaux ou chaque espace est rattache au mot qui le
    precede — c'est ce que consomme la mise en ligne.
    """
    if not text:
        return []
    words = []
    buf = []
    for char in text:
        buf.append(char)
        if char == " ":
            words.append("".join(buf))
            buf = []
    if buf:
        words.append("".join(buf))
    return words


def break_long_word(word, metrics, size, bold, available):
    """Coupe un mot plus large que la ligne. Renvoie (tete, reste).

    Sans ca, une URL de 300 caracteres depasserait indefiniment du cadre.
    """
    if available <= 0:
        return "", word
    low = 1
    high = len(word)
    best = 1
    while low <= high:
        middle = (low + high) // 2
        if metrics.measure(word[:middle], size, bold) <= available:
            best = middle
            low = middle + 1
        else:
            high = middle - 1
    return word[:best], word[best:]
