"""Analyse des feuilles de style.

Produit une `Stylesheet` : une liste de `Rule`, chacune portant des selecteurs
compiles (avec leur specificite) et un bloc de declarations. Les at-regles
sont traitees selon leur utilite pour le rendu : `@media` est evaluee contre
le viewport, `@import` est signalee au chargeur, le reste est ignore.
"""

from .values import split_shorthand

# Combinateurs de selecteurs.
DESCENDANT = " "
CHILD = ">"
ADJACENT = "+"
SIBLING = "~"


class Declaration:
    __slots__ = ("name", "value", "important")

    def __init__(self, name, value, important=False):
        self.name = name
        self.value = value
        self.important = important

    def __repr__(self):
        return "%s: %s%s" % (self.name, self.value, " !important" if self.important else "")


class SimpleSelector:
    """Un maillon : `div#id.classe[attr]:hover`."""

    __slots__ = ("tag", "ids", "classes", "attrs", "pseudo_classes", "combinator")

    def __init__(self):
        self.tag = ""
        self.ids = []
        self.classes = []
        self.attrs = []  # (nom, operateur, valeur)
        self.pseudo_classes = []
        self.combinator = DESCENDANT

    def __repr__(self):
        out = self.tag or "*"
        out += "".join("#" + i for i in self.ids)
        out += "".join("." + c for c in self.classes)
        out += "".join(":" + p for p in self.pseudo_classes)
        return out


class Selector:
    """Chaine de `SimpleSelector`, du plus externe au plus interne."""

    __slots__ = ("parts", "specificity", "source")

    def __init__(self, parts, source=""):
        self.parts = parts
        self.source = source
        self.specificity = self._compute_specificity()

    def _compute_specificity(self):
        ids = classes = tags = 0
        for part in self.parts:
            ids += len(part.ids)
            classes += len(part.classes) + len(part.attrs) + len(part.pseudo_classes)
            if part.tag and part.tag != "*":
                tags += 1
        return (ids, classes, tags)

    @property
    def key(self):
        """Cle d'indexation : le selecteur le plus a droite, pour filtrer vite."""
        if not self.parts:
            return None
        last = self.parts[-1]
        if last.ids:
            return ("#", last.ids[0])
        if last.classes:
            return (".", last.classes[0])
        if last.tag and last.tag != "*":
            return ("t", last.tag)
        return None

    def __repr__(self):
        return "Selector(%r, %s)" % (self.source, self.specificity)


class Rule:
    __slots__ = ("selectors", "declarations", "order", "origin")

    def __init__(self, selectors, declarations, order=0, origin="author"):
        self.selectors = selectors
        self.declarations = declarations
        self.order = order
        self.origin = origin

    def __repr__(self):
        return "Rule(%s, %d decls)" % (
            ", ".join(str(s) for s in self.selectors),
            len(self.declarations),
        )


class Stylesheet:
    __slots__ = ("rules", "imports", "origin")

    def __init__(self, rules=None, imports=None, origin="author"):
        self.rules = rules if rules is not None else []
        self.imports = imports if imports is not None else []
        self.origin = origin

    def extend(self, other):
        self.rules.extend(other.rules)
        self.imports.extend(other.imports)

    def __repr__(self):
        return "Stylesheet(%d rules)" % len(self.rules)


def _strip_comments(text):
    out = []
    i = 0
    length = len(text)
    while i < length:
        if text.startswith("/*", i):
            end = text.find("*/", i + 2)
            if end < 0:
                break
            i = end + 2
            # Un commentaire separe les jetons : on le remplace par un espace.
            out.append(" ")
            continue
        out.append(text[i])
        i += 1
    return "".join(out)


def parse_declarations(text):
    """Analyse le contenu d'un bloc `{...}` (sans les accolades)."""
    declarations = []
    for chunk in _split_top_level(text, ";"):
        chunk = chunk.strip()
        if not chunk:
            continue
        idx = chunk.find(":")
        if idx <= 0:
            continue
        name = chunk[:idx].strip().lower()
        value = chunk[idx + 1 :].strip()
        important = False
        lowered = value.lower()
        if lowered.endswith("!important"):
            value = value[: lowered.rfind("!important")].strip()
            important = True
        elif "!important" in lowered:
            value = value[: lowered.find("!important")].strip()
            important = True
        if name and value:
            declarations.append(Declaration(name, value, important))
    return declarations


def _split_top_level(text, separator):
    """Decoupe en ignorant les separateurs dans les parentheses et les quotes."""
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
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth = max(0, depth - 1)
        if ch == separator and depth == 0:
            parts.append("".join(buf))
            buf = []
            continue
        buf.append(ch)
    parts.append("".join(buf))
    return parts


def parse_selector(text):
    """Compile un selecteur unique. Renvoie None s'il est inutilisable."""
    text = text.strip()
    if not text:
        return None

    parts = []
    current = SimpleSelector()
    pending_combinator = DESCENDANT
    buf = ""
    mode = "tag"
    i = 0
    length = len(text)
    started = False

    def commit_buffer():
        # Le mode retombe toujours sur `tag` : sans ca, le fragment suivant
        # (`p` dans `.card > p`) serait range dans les classes du precedent.
        nonlocal buf, mode
        if buf:
            if mode == "tag":
                current.tag = buf.lower()
            elif mode == "#":
                current.ids.append(buf)
            elif mode == ".":
                current.classes.append(buf)
            elif mode == ":":
                current.pseudo_classes.append(buf.lower())
            buf = ""
        mode = "tag"

    def commit_part():
        nonlocal current, pending_combinator, started
        commit_buffer()
        current.combinator = pending_combinator
        parts.append(current)
        current = SimpleSelector()
        pending_combinator = DESCENDANT
        started = False

    while i < length:
        ch = text[i]

        if ch in " \t\n\r\f":
            # Un espace ne termine le maillon que s'il est suivi d'un autre maillon.
            j = i
            while j < length and text[j] in " \t\n\r\f":
                j += 1
            if j >= length:
                break
            nxt = text[j]
            if nxt in ">+~":
                i = j
                continue
            if started:
                commit_part()
                pending_combinator = DESCENDANT
            i = j
            continue

        if ch in ">+~":
            if started:
                commit_part()
            pending_combinator = ch
            i += 1
            continue

        if ch == "[":
            commit_buffer()
            end = text.find("]", i)
            if end < 0:
                return None
            current.attrs.append(_parse_attr(text[i + 1 : end]))
            started = True
            i = end + 1
            mode = "tag"
            continue

        if ch == ":":
            commit_buffer()
            # `::before` — pseudo-element : consomme puis ignore, ces boites
            # generees ne sont pas produites par ce moteur.
            if i + 1 < length and text[i + 1] == ":":
                i += 2
                while i < length and (text[i].isalnum() or text[i] == "-"):
                    i += 1
                started = True
                continue
            started = True
            i += 1
            # Les pseudo-classes fonctionnelles gardent leurs parentheses.
            name_start = i
            while i < length and (text[i].isalnum() or text[i] == "-"):
                i += 1
            name = text[name_start:i]
            if i < length and text[i] == "(":
                depth = 0
                while i < length:
                    if text[i] == "(":
                        depth += 1
                    elif text[i] == ")":
                        depth -= 1
                        if depth == 0:
                            i += 1
                            break
                    i += 1
                name += "()"
            current.pseudo_classes.append(name.lower())
            mode = "tag"
            buf = ""
            continue

        if ch in "#.":
            commit_buffer()
            mode = ch
            started = True
            i += 1
            continue

        if ch == "*":
            commit_buffer()
            current.tag = "*"
            started = True
            mode = "tag"
            i += 1
            continue

        buf += ch
        started = True
        i += 1

    commit_buffer()
    if started or not parts:
        current.combinator = pending_combinator
        parts.append(current)

    if not parts:
        return None
    return Selector(parts, text)


def _parse_attr(body):
    body = body.strip()
    for operator in ("|=", "^=", "$=", "*=", "~=", "="):
        idx = body.find(operator)
        if idx > 0:
            name = body[:idx].strip().lower()
            value = body[idx + len(operator) :].strip()
            if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
                value = value[1:-1]
            return (name, operator, value)
    return (body.lower(), "", "")


def parse(text, origin="author", viewport=(1024, 768)):
    """Analyse une feuille de style complete."""
    sheet = Stylesheet(origin=origin)
    if not text:
        return sheet
    text = _strip_comments(text)
    _parse_into(text, sheet, origin, viewport, order=[0])
    return sheet


def _parse_into(text, sheet, origin, viewport, order, media_ok=True):
    i = 0
    length = len(text)
    while i < length:
        # Saute les espaces.
        while i < length and text[i] in " \t\n\r\f":
            i += 1
        if i >= length:
            break

        if text[i] == "@":
            i = _parse_at_rule(text, i, sheet, origin, viewport, order, media_ok)
            continue

        brace = _find_block_start(text, i)
        if brace < 0:
            break
        prelude = text[i:brace].strip()
        end = _find_block_end(text, brace)
        body = text[brace + 1 : end]
        i = end + 1

        if not media_ok or not prelude:
            continue

        declarations = parse_declarations(body)
        if not declarations:
            continue
        selectors = []
        for chunk in _split_top_level(prelude, ","):
            selector = parse_selector(chunk)
            if selector is not None:
                selectors.append(selector)
        if selectors:
            sheet.rules.append(Rule(selectors, declarations, order[0], origin))
            order[0] += 1


def _find_block_start(text, start):
    quote = ""
    for i in range(start, len(text)):
        ch = text[i]
        if quote:
            if ch == quote:
                quote = ""
            continue
        if ch in "\"'":
            quote = ch
            continue
        if ch == "{":
            return i
        if ch == "}":
            # Accolade fermante orpheline : on resynchronise.
            return -1
    return -1


def _find_block_end(text, start):
    depth = 0
    quote = ""
    for i in range(start, len(text)):
        ch = text[i]
        if quote:
            if ch == quote:
                quote = ""
            continue
        if ch in "\"'":
            quote = ch
            continue
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return i
    return len(text)


def _parse_at_rule(text, start, sheet, origin, viewport, order, media_ok):
    length = len(text)
    i = start + 1
    name_start = i
    while i < length and (text[i].isalnum() or text[i] == "-"):
        i += 1
    name = text[name_start:i].lower()

    # At-regle sans bloc : se termine au `;`.
    semi = text.find(";", i)
    brace = _find_block_start(text, i)

    if brace < 0 or (0 <= semi < brace):
        prelude = text[i : semi if semi >= 0 else length].strip()
        if name == "import":
            url = _extract_url(prelude)
            if url:
                sheet.imports.append(url)
        return (semi + 1) if semi >= 0 else length

    prelude = text[i:brace].strip()
    end = _find_block_end(text, brace)
    body = text[brace + 1 : end]

    if name == "media":
        if media_ok and evaluate_media(prelude, viewport):
            _parse_into(body, sheet, origin, viewport, order, media_ok=True)
    elif name == "supports":
        # On suppose le support : mieux vaut styler que rendre une page nue.
        if media_ok:
            _parse_into(body, sheet, origin, viewport, order, media_ok=True)
    # `@font-face`, `@keyframes`, `@charset`… n'influencent pas ce moteur.
    return end + 1


def _extract_url(text):
    text = text.strip()
    if text.lower().startswith("url("):
        inner = text[4 : text.find(")")] if ")" in text else text[4:]
        return inner.strip().strip("\"'")
    if text[:1] in "\"'":
        quote = text[0]
        end = text.find(quote, 1)
        if end > 0:
            return text[1:end]
    return ""


def evaluate_media(query, viewport):
    """Evalue une requete media contre la largeur/hauteur du viewport.

    Gere `screen`, `all`, `min-width`, `max-width`, `min-height`,
    `max-height` et les listes separees par des virgules (un `ou`).
    """
    query = query.strip().lower()
    if not query:
        return True
    for alternative in query.split(","):
        if _evaluate_single_media(alternative.strip(), viewport):
            return True
    return False


def _evaluate_single_media(query, viewport):
    if not query:
        return True
    negated = False
    if query.startswith("not "):
        negated = True
        query = query[4:].strip()
    if query.startswith("only "):
        query = query[5:].strip()

    result = True
    for condition in query.split(" and "):
        condition = condition.strip().strip("()")
        if not condition:
            continue
        if condition in ("screen", "all"):
            continue
        if condition in ("print", "speech"):
            result = False
            break
        if ":" not in condition:
            # Fonctionnalite booleenne inconnue : on ne bloque pas le style.
            continue
        feature, _, raw = condition.partition(":")
        feature = feature.strip()
        raw = raw.strip()
        from .values import parse_length, to_pixels

        pixels = to_pixels(parse_length(raw), viewport=viewport)
        if pixels is None:
            continue
        if feature == "min-width" and viewport[0] < pixels:
            result = False
            break
        if feature == "max-width" and viewport[0] > pixels:
            result = False
            break
        if feature == "min-height" and viewport[1] < pixels:
            result = False
            break
        if feature == "max-height" and viewport[1] > pixels:
            result = False
            break
    return (not result) if negated else result
