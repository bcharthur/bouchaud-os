"""Couleurs CSS.

Les couleurs circulent dans le moteur sous forme d'entier `0xRRGGBB` — c'est
le format attendu par la couche de peinture et par le framebuffer de
Bouchaud OS. L'alpha est compose immediatement sur un fond fourni : le
compositeur du navigateur ne gere pas de canal alpha persistant.
"""

NAMED = {
    "transparent": None,
    "black": 0x000000, "silver": 0xC0C0C0, "gray": 0x808080, "grey": 0x808080,
    "white": 0xFFFFFF, "maroon": 0x800000, "red": 0xFF0000, "purple": 0x800080,
    "fuchsia": 0xFF00FF, "magenta": 0xFF00FF, "green": 0x008000, "lime": 0x00FF00,
    "olive": 0x808000, "yellow": 0xFFFF00, "navy": 0x000080, "blue": 0x0000FF,
    "teal": 0x008080, "aqua": 0x00FFFF, "cyan": 0x00FFFF, "orange": 0xFFA500,
    "aliceblue": 0xF0F8FF, "antiquewhite": 0xFAEBD7, "azure": 0xF0FFFF,
    "beige": 0xF5F5DC, "bisque": 0xFFE4C4, "blanchedalmond": 0xFFEBCD,
    "blueviolet": 0x8A2BE2, "brown": 0xA52A2A, "burlywood": 0xDEB887,
    "cadetblue": 0x5F9EA0, "chartreuse": 0x7FFF00, "chocolate": 0xD2691E,
    "coral": 0xFF7F50, "cornflowerblue": 0x6495ED, "cornsilk": 0xFFF8DC,
    "crimson": 0xDC143C, "darkblue": 0x00008B, "darkcyan": 0x008B8B,
    "darkgoldenrod": 0xB8860B, "darkgray": 0xA9A9A9, "darkgrey": 0xA9A9A9,
    "darkgreen": 0x006400, "darkkhaki": 0xBDB76B, "darkmagenta": 0x8B008B,
    "darkolivegreen": 0x556B2F, "darkorange": 0xFF8C00, "darkorchid": 0x9932CC,
    "darkred": 0x8B0000, "darksalmon": 0xE9967A, "darkseagreen": 0x8FBC8F,
    "darkslateblue": 0x483D8B, "darkslategray": 0x2F4F4F, "darkslategrey": 0x2F4F4F,
    "darkturquoise": 0x00CED1, "darkviolet": 0x9400D3, "deeppink": 0xFF1493,
    "deepskyblue": 0x00BFFF, "dimgray": 0x696969, "dimgrey": 0x696969,
    "dodgerblue": 0x1E90FF, "firebrick": 0xB22222, "floralwhite": 0xFFFAF0,
    "forestgreen": 0x228B22, "gainsboro": 0xDCDCDC, "ghostwhite": 0xF8F8FF,
    "gold": 0xFFD700, "goldenrod": 0xDAA520, "greenyellow": 0xADFF2F,
    "honeydew": 0xF0FFF0, "hotpink": 0xFF69B4, "indianred": 0xCD5C5C,
    "indigo": 0x4B0082, "ivory": 0xFFFFF0, "khaki": 0xF0E68C,
    "lavender": 0xE6E6FA, "lavenderblush": 0xFFF0F5, "lawngreen": 0x7CFC00,
    "lemonchiffon": 0xFFFACD, "lightblue": 0xADD8E6, "lightcoral": 0xF08080,
    "lightcyan": 0xE0FFFF, "lightgoldenrodyellow": 0xFAFAD2, "lightgray": 0xD3D3D3,
    "lightgrey": 0xD3D3D3, "lightgreen": 0x90EE90, "lightpink": 0xFFB6C1,
    "lightsalmon": 0xFFA07A, "lightseagreen": 0x20B2AA, "lightskyblue": 0x87CEFA,
    "lightslategray": 0x778899, "lightslategrey": 0x778899, "lightsteelblue": 0xB0C4DE,
    "lightyellow": 0xFFFFE0, "limegreen": 0x32CD32, "linen": 0xFAF0E6,
    "mediumaquamarine": 0x66CDAA, "mediumblue": 0x0000CD, "mediumorchid": 0xBA55D3,
    "mediumpurple": 0x9370DB, "mediumseagreen": 0x3CB371, "mediumslateblue": 0x7B68EE,
    "mediumspringgreen": 0x00FA9A, "mediumturquoise": 0x48D1CC,
    "mediumvioletred": 0xC71585, "midnightblue": 0x191970, "mintcream": 0xF5FFFA,
    "mistyrose": 0xFFE4E1, "moccasin": 0xFFE4B5, "navajowhite": 0xFFDEAD,
    "oldlace": 0xFDF5E6, "olivedrab": 0x6B8E23, "orangered": 0xFF4500,
    "orchid": 0xDA70D6, "palegoldenrod": 0xEEE8AA, "palegreen": 0x98FB98,
    "paleturquoise": 0xAFEEEE, "palevioletred": 0xDB7093, "papayawhip": 0xFFEFD5,
    "peachpuff": 0xFFDAB9, "peru": 0xCD853F, "pink": 0xFFC0CB, "plum": 0xDDA0DD,
    "powderblue": 0xB0E0E6, "rosybrown": 0xBC8F8F, "royalblue": 0x4169E1,
    "saddlebrown": 0x8B4513, "salmon": 0xFA8072, "sandybrown": 0xF4A460,
    "seagreen": 0x2E8B57, "seashell": 0xFFF5EE, "sienna": 0xA0522D,
    "skyblue": 0x87CEEB, "slateblue": 0x6A5ACD, "slategray": 0x708090,
    "slategrey": 0x708090, "snow": 0xFFFAFA, "springgreen": 0x00FF7F,
    "steelblue": 0x4682B4, "tan": 0xD2B48C, "thistle": 0xD8BFD8,
    "tomato": 0xFF6347, "turquoise": 0x40E0D0, "violet": 0xEE82EE,
    "wheat": 0xF5DEB3, "whitesmoke": 0xF5F5F5, "yellowgreen": 0x9ACD32,
    "rebeccapurple": 0x663399,
}


def rgb(r, g, b):
    """Assemble trois composantes 0-255 en entier `0xRRGGBB`."""
    r = 0 if r < 0 else (255 if r > 255 else int(r))
    g = 0 if g < 0 else (255 if g > 255 else int(g))
    b = 0 if b < 0 else (255 if b > 255 else int(b))
    return (r << 16) | (g << 8) | b


def components(value):
    """Decompose un `0xRRGGBB` en (r, g, b)."""
    return ((value >> 16) & 0xFF, (value >> 8) & 0xFF, value & 0xFF)


def blend(fg, bg, alpha):
    """Compose `fg` sur `bg` avec `alpha` dans [0, 1]."""
    if alpha >= 1.0:
        return fg
    if alpha <= 0.0:
        return bg
    fr, fg_, fb = components(fg)
    br, bg_, bb = components(bg)
    return rgb(
        br + (fr - br) * alpha,
        bg_ + (fg_ - bg_) * alpha,
        bb + (fb - bb) * alpha,
    )


def parse(text, background=0xFFFFFF):
    """Analyse une couleur CSS. Renvoie `0xRRGGBB`, ou None si transparent."""
    if text is None:
        return None
    text = text.strip().lower()
    if not text:
        return None

    if text in NAMED:
        return NAMED[text]

    if text.startswith("#"):
        return _parse_hex(text[1:])

    if text.startswith("rgb"):
        return _parse_rgb(text, background)

    if text.startswith("hsl"):
        return _parse_hsl(text, background)

    # `currentcolor` et les mots-cles systeme retombent sur le texte par defaut.
    if text == "currentcolor":
        return None
    return None


def _parse_hex(digits):
    # On tolere les chiffres invalides en les ignorant, comme les navigateurs.
    if not digits:
        return None
    try:
        if len(digits) == 3:
            r, g, b = (int(c * 2, 16) for c in digits)
            return rgb(r, g, b)
        if len(digits) == 4:
            r, g, b = (int(c * 2, 16) for c in digits[:3])
            return rgb(r, g, b)
        if len(digits) >= 6:
            return rgb(
                int(digits[0:2], 16), int(digits[2:4], 16), int(digits[4:6], 16)
            )
    except ValueError:
        return None
    return None


def _split_args(text):
    start = text.find("(")
    end = text.rfind(")")
    if start < 0:
        return []
    body = text[start + 1 : end if end > start else len(text)]
    # CSS Color 4 autorise les espaces et le `/` comme separateurs.
    body = body.replace(",", " ").replace("/", " ")
    return [part for part in body.split() if part]


def _channel(token):
    token = token.strip()
    if token.endswith("%"):
        try:
            return float(token[:-1]) * 255.0 / 100.0
        except ValueError:
            return 0.0
    try:
        return float(token)
    except ValueError:
        return 0.0


def _alpha(token):
    token = token.strip()
    try:
        if token.endswith("%"):
            value = float(token[:-1]) / 100.0
        else:
            value = float(token)
    except ValueError:
        return 1.0
    return 0.0 if value < 0.0 else (1.0 if value > 1.0 else value)


def _parse_rgb(text, background):
    args = _split_args(text)
    if len(args) < 3:
        return None
    color = rgb(_channel(args[0]), _channel(args[1]), _channel(args[2]))
    if len(args) >= 4:
        return blend(color, background, _alpha(args[3]))
    return color


def _parse_hsl(text, background):
    args = _split_args(text)
    if len(args) < 3:
        return None
    try:
        hue = float(args[0].rstrip("dget")) % 360.0
    except ValueError:
        return None
    sat = _percent(args[1])
    light = _percent(args[2])
    color = hsl_to_rgb(hue, sat, light)
    if len(args) >= 4:
        return blend(color, background, _alpha(args[3]))
    return color


def _percent(token):
    token = token.strip().rstrip("%")
    try:
        return max(0.0, min(1.0, float(token) / 100.0))
    except ValueError:
        return 0.0


def hsl_to_rgb(hue, sat, light):
    """Conversion HSL -> RGB (CSS Color 3, section 4.2.4)."""
    if sat == 0.0:
        level = light * 255.0
        return rgb(level, level, level)
    chroma = (1.0 - abs(2.0 * light - 1.0)) * sat
    secondary = chroma * (1.0 - abs((hue / 60.0) % 2.0 - 1.0))
    match = light - chroma / 2.0
    sector = int(hue // 60) % 6
    table = (
        (chroma, secondary, 0.0),
        (secondary, chroma, 0.0),
        (0.0, chroma, secondary),
        (0.0, secondary, chroma),
        (secondary, 0.0, chroma),
        (chroma, 0.0, secondary),
    )
    r, g, b = table[sector]
    return rgb((r + match) * 255.0, (g + match) * 255.0, (b + match) * 255.0)


def luminance(value):
    """Luminance perceptuelle approximative (0-255), pour choisir un contraste."""
    r, g, b = components(value)
    return (r * 299 + g * 587 + b * 114) // 1000


def readable_on(background):
    """Noir ou blanc, selon ce qui se lit le mieux sur `background`."""
    return 0x000000 if luminance(background) > 140 else 0xFFFFFF
