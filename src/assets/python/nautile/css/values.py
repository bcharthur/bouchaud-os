"""Valeurs et unites CSS.

Conversion des longueurs en pixels CSS. Le moteur travaille en entiers de
pixels : c'est ce que consomment le layout et le framebuffer, et cela evite
les erreurs d'arrondi cumulees sur les grandes pages.
"""

# Taille de police racine par defaut (`1rem`).
ROOT_FONT_SIZE = 16

# Facteurs de conversion vers le pixel CSS (1in = 96px par definition).
_ABSOLUTE = {
    "px": 1.0,
    "in": 96.0,
    "cm": 96.0 / 2.54,
    "mm": 96.0 / 25.4,
    "q": 96.0 / 101.6,
    "pt": 96.0 / 72.0,
    "pc": 16.0,
}

# Mots-cles de taille de police (CSS Fonts, table des tailles absolues).
ABSOLUTE_FONT_SIZES = {
    "xx-small": 9, "x-small": 10, "small": 13, "medium": 16,
    "large": 18, "x-large": 24, "xx-large": 32, "xxx-large": 48,
}


class Length:
    """Longueur analysee, resolue plus tard contre un contexte."""

    __slots__ = ("value", "unit")

    def __init__(self, value, unit="px"):
        self.value = value
        self.unit = unit

    @property
    def is_relative(self):
        return self.unit in ("%", "em", "rem", "ex", "ch", "vw", "vh", "vmin", "vmax")

    def __repr__(self):
        return "Length(%g%s)" % (self.value, self.unit)


def parse_length(text):
    """Analyse `12px`, `1.5em`, `50%`… Renvoie une `Length` ou None."""
    if text is None:
        return None
    text = text.strip().lower()
    if not text:
        return None
    if text == "0":
        return Length(0.0, "px")
    if text.endswith("%"):
        try:
            return Length(float(text[:-1]), "%")
        except ValueError:
            return None

    for unit in ("rem", "vmin", "vmax", "px", "em", "ex", "ch", "vw", "vh",
                 "cm", "mm", "in", "pt", "pc", "q"):
        if text.endswith(unit):
            head = text[: -len(unit)].strip()
            try:
                return Length(float(head), unit)
            except ValueError:
                return None

    # Nombre nu : tolere, interprete en pixels (courant dans les attributs HTML).
    try:
        return Length(float(text), "px")
    except ValueError:
        return None


def to_pixels(length, font_size=ROOT_FONT_SIZE, percent_base=0, viewport=(1024, 768)):
    """Resout une `Length` en pixels entiers.

    `font_size` sert aux unites relatives a la police, `percent_base` aux
    pourcentages, `viewport` aux unites `vw`/`vh`.
    """
    if length is None:
        return None
    unit = length.unit
    value = length.value

    if unit in _ABSOLUTE:
        return int(round(value * _ABSOLUTE[unit]))
    if unit == "%":
        return int(round(value * percent_base / 100.0))
    if unit == "em":
        return int(round(value * font_size))
    if unit == "rem":
        return int(round(value * ROOT_FONT_SIZE))
    if unit == "ex":
        # Approximation usuelle : la hauteur d'x vaut environ la moitie du cadratin.
        return int(round(value * font_size * 0.5))
    if unit == "ch":
        # Largeur du `0` : ~0.5em pour les polices proportionnelles courantes.
        return int(round(value * font_size * 0.5))
    if unit == "vw":
        return int(round(value * viewport[0] / 100.0))
    if unit == "vh":
        return int(round(value * viewport[1] / 100.0))
    if unit == "vmin":
        return int(round(value * min(viewport) / 100.0))
    if unit == "vmax":
        return int(round(value * max(viewport) / 100.0))
    return int(round(value))


def resolve(text, font_size=ROOT_FONT_SIZE, percent_base=0, viewport=(1024, 768)):
    """Raccourci : analyse puis resout en pixels. None si inanalysable."""
    return to_pixels(parse_length(text), font_size, percent_base, viewport)


def parse_font_size(text, parent_size=ROOT_FONT_SIZE, viewport=(1024, 768)):
    """Taille de police : mots-cles absolus, relatifs, longueurs, pourcentages."""
    if not text:
        return parent_size
    text = text.strip().lower()
    if text in ABSOLUTE_FONT_SIZES:
        return ABSOLUTE_FONT_SIZES[text]
    if text == "larger":
        return int(round(parent_size * 1.2))
    if text == "smaller":
        return int(round(parent_size / 1.2))
    if text == "inherit":
        return parent_size
    size = to_pixels(parse_length(text), parent_size, parent_size, viewport)
    if size is None or size <= 0:
        return parent_size
    # Garde-fou : une page hostile ne doit pas faire exploser le layout.
    return min(size, 400)


def parse_number(text, default=0.0):
    try:
        return float(str(text).strip())
    except (TypeError, ValueError):
        return default


def split_shorthand(text):
    """Decoupe une valeur composee en respectant les parentheses et quotes.

    `1px solid rgb(0, 0, 0)` donne trois morceaux, pas cinq.
    """
    if not text:
        return []
    parts = []
    buf = []
    depth = 0
    quote = ""
    for ch in text:
        if quote:
            buf.append(ch)
            if ch == quote:
                quote = ""
            continue
        if ch in "\"'":
            quote = ch
            buf.append(ch)
            continue
        if ch == "(":
            depth += 1
            buf.append(ch)
            continue
        if ch == ")":
            depth = max(0, depth - 1)
            buf.append(ch)
            continue
        if ch in " \t\n\r\f" and depth == 0:
            if buf:
                parts.append("".join(buf))
                buf = []
            continue
        buf.append(ch)
    if buf:
        parts.append("".join(buf))
    return parts


def expand_box(parts):
    """Etend un raccourci de boite en (haut, droite, bas, gauche)."""
    count = len(parts)
    if count == 0:
        return None
    if count == 1:
        return (parts[0], parts[0], parts[0], parts[0])
    if count == 2:
        return (parts[0], parts[1], parts[0], parts[1])
    if count == 3:
        return (parts[0], parts[1], parts[2], parts[1])
    return (parts[0], parts[1], parts[2], parts[3])
