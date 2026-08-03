"""References de caracteres HTML.

Table des entites nommees courantes plus le decodage numerique (`&#38;`,
`&#x26;`). La table complete du standard compte plus de 2200 entrees ; on
embarque celles que le Web reel utilise, le reste passe en litteral — le
comportement des navigateurs pour une entite inconnue.
"""

NAMED = {
    "amp": "&", "lt": "<", "gt": ">", "quot": '"', "apos": "'",
    "nbsp": " ", "ensp": " ", "emsp": " ", "thinsp": " ",
    "copy": "©", "reg": "®", "trade": "™", "deg": "°",
    "plusmn": "±", "sup2": "²", "sup3": "³", "micro": "µ",
    "para": "¶", "middot": "·", "frac12": "½", "frac14": "¼",
    "frac34": "¾", "times": "×", "divide": "÷", "minus": "−",
    "ndash": "–", "mdash": "—", "lsquo": "‘", "rsquo": "’",
    "ldquo": "“", "rdquo": "”", "sbquo": "‚", "bdquo": "„",
    "dagger": "†", "Dagger": "‡", "bull": "•", "hellip": "…",
    "permil": "‰", "prime": "′", "Prime": "″", "lsaquo": "‹",
    "rsaquo": "›", "oline": "‾", "frasl": "⁄", "euro": "€",
    "pound": "£", "yen": "¥", "cent": "¢", "curren": "¤",
    "sect": "§", "uml": "¨", "ordf": "ª", "laquo": "«",
    "not": "¬", "shy": "­", "macr": "¯", "acute": "´",
    "cedil": "¸", "ordm": "º", "raquo": "»", "iquest": "¿",
    "iexcl": "¡", "brvbar": "¦", "szlig": "ß",
    "agrave": "à", "aacute": "á", "acirc": "â", "atilde": "ã",
    "auml": "ä", "aring": "å", "aelig": "æ", "ccedil": "ç",
    "egrave": "è", "eacute": "é", "ecirc": "ê", "euml": "ë",
    "igrave": "ì", "iacute": "í", "icirc": "î", "iuml": "ï",
    "ntilde": "ñ", "ograve": "ò", "oacute": "ó", "ocirc": "ô",
    "otilde": "õ", "ouml": "ö", "oslash": "ø", "ugrave": "ù",
    "uacute": "ú", "ucirc": "û", "uuml": "ü", "yacute": "ý",
    "yuml": "ÿ",
    "Agrave": "À", "Aacute": "Á", "Acirc": "Â", "Atilde": "Ã",
    "Auml": "Ä", "Aring": "Å", "AElig": "Æ", "Ccedil": "Ç",
    "Egrave": "È", "Eacute": "É", "Ecirc": "Ê", "Euml": "Ë",
    "Igrave": "Ì", "Iacute": "Í", "Icirc": "Î", "Iuml": "Ï",
    "Ntilde": "Ñ", "Ograve": "Ò", "Oacute": "Ó", "Ocirc": "Ô",
    "Otilde": "Õ", "Ouml": "Ö", "Oslash": "Ø", "Ugrave": "Ù",
    "Uacute": "Ú", "Ucirc": "Û", "Uuml": "Ü", "Yacute": "Ý",
    "larr": "←", "uarr": "↑", "rarr": "→", "darr": "↓",
    "harr": "↔", "crarr": "↵", "lArr": "⇐", "rArr": "⇒",
    "hArr": "⇔", "infin": "∞", "ne": "≠", "le": "≤",
    "ge": "≥", "asymp": "≈", "equiv": "≡", "sum": "∑",
    "prod": "∏", "radic": "√", "int": "∫", "part": "∂",
    "alpha": "α", "beta": "β", "gamma": "γ", "delta": "δ",
    "epsilon": "ε", "lambda": "λ", "mu": "μ", "pi": "π",
    "sigma": "σ", "tau": "τ", "phi": "φ", "omega": "ω",
    "Alpha": "Α", "Beta": "Β", "Gamma": "Γ", "Delta": "Δ",
    "Lambda": "Λ", "Pi": "Π", "Sigma": "Σ", "Omega": "Ω",
    "spades": "♠", "clubs": "♣", "hearts": "♥", "diams": "♦",
    "loz": "◊", "star": "☆", "check": "✓", "cross": "✗",
}

# Remappage des references numeriques Windows-1252, exige par le standard :
# `&#149;` doit donner un point median et non le caractere de controle U+0095.
_CP1252 = {
    0x80: "€", 0x82: "‚", 0x83: "ƒ", 0x84: "„",
    0x85: "…", 0x86: "†", 0x87: "‡", 0x88: "ˆ",
    0x89: "‰", 0x8A: "Š", 0x8B: "‹", 0x8C: "Œ",
    0x8E: "Ž", 0x91: "‘", 0x92: "’", 0x93: "“",
    0x94: "”", 0x95: "•", 0x96: "–", 0x97: "—",
    0x98: "˜", 0x99: "™", 0x9A: "š", 0x9B: "›",
    0x9C: "œ", 0x9E: "ž", 0x9F: "Ÿ",
}

_MAX_NAME_LEN = max(len(name) for name in NAMED)


def _codepoint_to_char(value):
    if value in _CP1252:
        return _CP1252[value]
    if value == 0 or value > 0x10FFFF or 0xD800 <= value <= 0xDFFF:
        return "�"
    return chr(value)


def decode(text):
    """Remplace toutes les references de caracteres d'une chaine."""
    if "&" not in text:
        return text
    out = []
    i = 0
    length = len(text)
    while i < length:
        ch = text[i]
        if ch != "&":
            out.append(ch)
            i += 1
            continue
        semi = text.find(";", i + 1, i + _MAX_NAME_LEN + 3)
        if semi < 0:
            out.append(ch)
            i += 1
            continue
        body = text[i + 1 : semi]
        if body.startswith("#"):
            digits = body[1:]
            try:
                if digits[:1] in ("x", "X"):
                    value = int(digits[1:], 16)
                else:
                    value = int(digits, 10)
            except ValueError:
                out.append(ch)
                i += 1
                continue
            out.append(_codepoint_to_char(value))
            i = semi + 1
            continue
        replacement = NAMED.get(body)
        if replacement is None:
            # Entite inconnue : on laisse le texte tel quel, comme les navigateurs.
            out.append(ch)
            i += 1
            continue
        out.append(replacement)
        i = semi + 1
    return "".join(out)
