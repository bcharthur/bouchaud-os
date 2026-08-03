"""Cascade et calcul des styles.

Produit un `ComputedStyle` par element : la cascade (origine, specificite,
ordre) puis l'heritage. Les proprietes sont resolues en valeurs directement
consommables par le layout — des pixels entiers, des couleurs `0xRRGGBB`,
des mots-cles normalises.
"""

import os

from .css import color as csscolor
from .css import parser as cssparser
from .css import select as cssselect
from .css.values import (
    ROOT_FONT_SIZE,
    expand_box,
    parse_font_size,
    parse_length,
    split_shorthand,
    to_pixels,
)
from .dom import Element, Text

_UA_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "ua.css")
_ua_cache = None

# Proprietes heritees par defaut (CSS 2.1, annexe F).
INHERITED = frozenset(
    {
        "color", "font-size", "font-family", "font-style", "font-weight",
        "line-height", "text-align", "text-indent", "text-transform",
        "letter-spacing", "word-spacing", "white-space", "visibility",
        "list-style-type", "list-style-position", "cursor", "direction",
    }
)

# Ordre des origines dans la cascade : plus grand gagne.
ORIGIN_WEIGHT = {"user-agent": 0, "user": 1, "author": 2, "inline": 3}


def user_agent_stylesheet():
    """Feuille user-agent, analysee une seule fois par processus."""
    global _ua_cache
    if _ua_cache is None:
        try:
            with open(_UA_PATH, "r", encoding="utf-8") as handle:
                source = handle.read()
        except OSError:
            source = ""
        _ua_cache = cssparser.parse(source, origin="user-agent")
    return _ua_cache


class ComputedStyle:
    """Valeurs calculees d'un element, resolues en unites de rendu."""

    __slots__ = (
        "display", "position", "color", "background_color", "font_size",
        "bold", "italic", "underline", "line_through", "monospace",
        "text_align", "line_height", "white_space", "visibility",
        "margin", "padding", "border_width", "border_color",
        "width", "height", "min_width", "max_width", "min_height", "max_height",
        "z_index", "overflow", "opacity", "list_style_type", "text_indent",
        "float_", "clear", "vertical_align", "border_radius", "cursor",
        "raw",
    )

    def __init__(self):
        self.display = "inline"
        self.position = "static"
        self.color = 0x202124
        self.background_color = None
        self.font_size = ROOT_FONT_SIZE
        self.bold = False
        self.italic = False
        self.underline = False
        self.line_through = False
        self.monospace = False
        self.text_align = "left"
        self.line_height = int(ROOT_FONT_SIZE * 1.45)
        self.white_space = "normal"
        self.visibility = "visible"
        self.margin = (0, 0, 0, 0)  # haut, droite, bas, gauche
        self.padding = (0, 0, 0, 0)
        self.border_width = (0, 0, 0, 0)
        self.border_color = (None, None, None, None)
        self.width = None
        self.height = None
        self.min_width = None
        self.max_width = None
        self.min_height = None
        self.max_height = None
        self.z_index = 0
        self.overflow = "visible"
        self.opacity = 1.0
        self.list_style_type = "disc"
        self.text_indent = 0
        self.float_ = "none"
        self.clear = "none"
        self.vertical_align = "baseline"
        self.border_radius = 0
        self.cursor = "auto"
        self.raw = {}

    @property
    def is_block(self):
        return self.display in ("block", "list-item", "flex", "grid", "table",
                                "table-row", "table-cell", "table-row-group",
                                "table-header-group", "table-footer-group",
                                "table-caption")

    @property
    def is_none(self):
        return self.display == "none"

    @property
    def is_inline(self):
        return self.display in ("inline", "inline-block", "inline-flex")

    def copy_inherited(self):
        """Nouveau style portant les proprietes heritables de celui-ci."""
        child = ComputedStyle()
        child.color = self.color
        child.font_size = self.font_size
        child.bold = self.bold
        child.italic = self.italic
        child.underline = self.underline
        child.line_through = self.line_through
        child.monospace = self.monospace
        child.text_align = self.text_align
        child.line_height = self.line_height
        child.white_space = self.white_space
        child.visibility = self.visibility
        child.list_style_type = self.list_style_type
        child.cursor = self.cursor
        # Le fond n'est pas herite : il transparait naturellement au rendu.
        return child

    def __repr__(self):
        return "ComputedStyle(%s, %dpx, #%06x)" % (
            self.display,
            self.font_size,
            self.color,
        )


class StyleEngine:
    """Applique une liste de feuilles a un document."""

    def __init__(self, viewport=(1024, 768), context=None):
        self.viewport = viewport
        self.context = context if context is not None else cssselect.MatchContext()
        self.index = cssselect.RuleIndex()
        self._sheets = []

    def add_stylesheet(self, sheet):
        self._sheets.append(sheet)
        self.index.add_stylesheet(sheet)

    def add_source(self, source, origin="author"):
        sheet = cssparser.parse(source, origin=origin, viewport=self.viewport)
        self.add_stylesheet(sheet)
        return sheet

    def style_document(self, document):
        """Calcule et attache le style de chaque element du document."""
        root_style = ComputedStyle()
        root_style.display = "block"
        for child in document.children:
            if isinstance(child, Element):
                self._style_element(child, root_style)
        return document

    def _style_element(self, element, parent_style):
        style = self.compute(element, parent_style)
        element.style = style
        # `display: none` coupe la branche : ni style ni layout pour la suite.
        if style.is_none:
            for node in element.descendants():
                if isinstance(node, Element):
                    node.style = None
            return
        for child in element.children:
            if isinstance(child, Element):
                self._style_element(child, style)

    # -- Cascade -----------------------------------------------------------

    def collect_declarations(self, element):
        """Toutes les declarations applicables, deja triees par poids."""
        matched = []
        for selector, rule in self.index.candidates(element):
            if cssselect.matches(element, selector, self.context):
                weight = (
                    ORIGIN_WEIGHT.get(rule.origin, 2),
                    selector.specificity[0],
                    selector.specificity[1],
                    selector.specificity[2],
                    rule.order,
                )
                matched.append((weight, rule.declarations))

        # Les attributs de presentation HTML (`bgcolor`, `width`…) se placent
        # entre la feuille user-agent et l'auteur : ils habillent un document
        # sans CSS, mais la moindre regle auteur les remplace.
        hints = _presentational_hints(element)
        if hints:
            matched.append(
                (
                    (ORIGIN_WEIGHT["user"], 0, 0, 0, -1),
                    [cssparser.Declaration(name, value) for name, value in hints.items()],
                )
            )

        matched.sort(key=lambda item: item[0])

        ordered = []
        important = []
        for weight, declarations in matched:
            for declaration in declarations:
                if declaration.important:
                    important.append((weight, declaration))
                else:
                    ordered.append(declaration)

        # L'attribut `style` bat toutes les regles non-!important.
        inline = element.attrs.get("style")
        if inline:
            for declaration in cssparser.parse_declarations(inline):
                if declaration.important:
                    important.append(((ORIGIN_WEIGHT["inline"], 9, 9, 9, 1 << 30), declaration))
                else:
                    ordered.append(declaration)

        # Les !important s'appliquent en dernier, dans leur propre ordre.
        important.sort(key=lambda item: item[0])
        ordered.extend(declaration for _, declaration in important)
        return ordered

    def compute(self, element, parent_style):
        style = parent_style.copy_inherited()
        style.display = _default_display(element)

        # Les raccourcis sont eclates en proprietes atomiques au fil de la
        # cascade. Sans ca, `margin: 4px` d'une regle auteur ne pourrait pas
        # remplacer le `margin-top` de la feuille user-agent : un dictionnaire
        # plat de noms de proprietes perd l'ordre relatif des deux.
        properties = {}
        for declaration in self.collect_declarations(element):
            for name, value in _expand_declaration(declaration.name, declaration.value):
                properties[name] = value

        style.raw = properties
        self._apply(style, properties, parent_style)
        return style

    def _apply(self, style, properties, parent_style):
        viewport = self.viewport
        parent_font = parent_style.font_size

        # La taille de police vient en premier : les `em` des autres
        # proprietes se resolvent contre elle.
        if "font-size" in properties:
            style.font_size = parse_font_size(properties["font-size"], parent_font, viewport)

        font = style.font_size

        def length(value, base=0):
            return to_pixels(parse_length(value), font, base, viewport)

        for name, value in properties.items():
            lowered = value.strip().lower()
            if lowered == "inherit":
                continue

            if name == "display":
                style.display = lowered
            elif name == "position":
                style.position = lowered
            elif name == "color":
                parsed = csscolor.parse(value)
                if parsed is not None:
                    style.color = parsed
            elif name == "background-color":
                style.background_color = csscolor.parse(value)
            elif name == "font-weight":
                style.bold = _is_bold(lowered)
            elif name == "font-style":
                style.italic = lowered in ("italic", "oblique")
            elif name == "font-family":
                style.monospace = "monospace" in lowered or "courier" in lowered
            elif name == "text-decoration" or name == "text-decoration-line":
                style.underline = "underline" in lowered
                style.line_through = "line-through" in lowered
                if "none" in lowered:
                    style.underline = False
                    style.line_through = False
            elif name == "text-align":
                if lowered in ("left", "right", "center", "justify", "start", "end"):
                    style.text_align = {"start": "left", "end": "right"}.get(lowered, lowered)
            elif name == "line-height":
                style.line_height = _parse_line_height(value, font)
            elif name == "white-space":
                style.white_space = lowered
            elif name == "visibility":
                style.visibility = lowered
            elif name == "opacity":
                try:
                    style.opacity = max(0.0, min(1.0, float(lowered)))
                except ValueError:
                    pass
            elif name == "overflow":
                style.overflow = lowered.split()[0] if lowered else "visible"
            elif name == "z-index":
                try:
                    style.z_index = int(lowered)
                except ValueError:
                    style.z_index = 0
            elif name == "float":
                style.float_ = lowered
            elif name == "clear":
                style.clear = lowered
            elif name == "vertical-align":
                style.vertical_align = lowered
            elif name == "cursor":
                style.cursor = lowered
            elif name == "list-style-type":
                style.list_style_type = lowered
            elif name == "text-indent":
                style.text_indent = length(value, viewport[0]) or 0
            elif name == "border-radius":
                first = split_shorthand(value)
                style.border_radius = (length(first[0]) or 0) if first else 0
            elif name == "width":
                style.width = length(value, viewport[0])
            elif name == "height":
                style.height = length(value, viewport[1])
            elif name == "min-width":
                style.min_width = length(value, viewport[0])
            elif name == "max-width":
                style.max_width = length(value, viewport[0])
            elif name == "min-height":
                style.min_height = length(value, viewport[1])
            elif name == "max-height":
                style.max_height = length(value, viewport[1])

        _apply_box(style, "margin", properties, length, viewport)
        _apply_box(style, "padding", properties, length, viewport)
        _apply_border(style, properties, length)


def _apply_box(style, prefix, properties, length, viewport):
    """Resout margin/padding depuis les quatre proprietes atomiques."""
    sides = list(getattr(style, prefix))
    for index, side in enumerate(("top", "right", "bottom", "left")):
        raw = properties.get("%s-%s" % (prefix, side))
        if raw is None:
            continue
        if raw.strip().lower() == "auto":
            # `auto` vaut zero ici ; le layout lit `raw` pour centrer.
            sides[index] = 0
            continue
        sides[index] = length(raw, viewport[0]) or 0
    setattr(style, prefix, tuple(sides))


def _apply_border(style, properties, length):
    """Resout les bordures depuis les proprietes atomiques par cote.

    La largeur utilisee est nulle tant qu'aucun style de trait n'est declare :
    c'est la regle CSS (`border-style` vaut `none` par defaut), et c'est ce
    qui evite de peindre un cadre autour de tout element portant seulement
    un `border-width`.
    """
    widths = list(style.border_width)
    colors = list(style.border_color)

    for index, side in enumerate(("top", "right", "bottom", "left")):
        line_style = properties.get("border-%s-style" % side, "none").strip().lower()
        raw_color = properties.get("border-%s-color" % side)
        if raw_color is not None:
            colors[index] = csscolor.parse(raw_color)

        if line_style in ("none", "hidden", ""):
            widths[index] = 0
            continue

        raw_width = properties.get("border-%s-width" % side)
        widths[index] = _border_width(raw_width, length)

    style.border_width = tuple(widths)
    style.border_color = tuple(colors)


# Largeurs nommees de bordure (CSS Backgrounds, valeurs usuelles des moteurs).
_BORDER_WIDTH_KEYWORDS = {"thin": 1, "medium": 3, "thick": 5}

_BORDER_STYLES = frozenset(
    {"none", "hidden", "solid", "dashed", "dotted", "double", "groove",
     "ridge", "inset", "outset"}
)


def _border_width(raw, length):
    if raw is None:
        # Style declare sans largeur : `medium`, comme le veut la cascade CSS.
        return 3
    lowered = raw.strip().lower()
    if lowered in _BORDER_WIDTH_KEYWORDS:
        return _BORDER_WIDTH_KEYWORDS[lowered]
    width = length(raw)
    return width if width is not None else 3


def _split_border(value):
    """Eclate `1px solid red` en (largeur, style, couleur) — None si absent."""
    width = line_style = colour = None
    for token in split_shorthand(value):
        lowered = token.strip().lower()
        if lowered in _BORDER_STYLES:
            line_style = lowered
            continue
        if lowered in _BORDER_WIDTH_KEYWORDS:
            width = lowered
            continue
        if csscolor.parse(token) is not None:
            colour = token
            continue
        if parse_length(token) is not None:
            width = token
    return width, line_style, colour


def _expand_declaration(name, value):
    """Traduit un raccourci en proprietes atomiques.

    Renvoie une liste de paires (nom, valeur). Une propriete deja atomique se
    renvoie elle-meme. C'est ce qui rend la cascade correcte entre un
    raccourci et une propriete longue declarees dans des regles differentes.
    """
    sides = ("top", "right", "bottom", "left")

    if name in ("margin", "padding"):
        parts = expand_box(split_shorthand(value))
        if not parts:
            return []
        return [("%s-%s" % (name, side), parts[i]) for i, side in enumerate(sides)]

    if name in ("border-width", "border-style", "border-color"):
        suffix = name.split("-", 1)[1]
        parts = expand_box(split_shorthand(value))
        if not parts:
            return []
        out = [("border-%s-%s" % (side, suffix), parts[i]) for i, side in enumerate(sides)]
        if suffix == "width":
            # Une largeur seule reste invisible sans style : on ne suppose rien.
            return out
        return out

    if name == "border":
        width, line_style, colour = _split_border(value)
        out = []
        for side in sides:
            out.append(("border-%s-style" % side, line_style or "none"))
            if width is not None:
                out.append(("border-%s-width" % side, width))
            if colour is not None:
                out.append(("border-%s-color" % side, colour))
        return out

    if name in ("border-top", "border-right", "border-bottom", "border-left"):
        side = name.split("-", 1)[1]
        width, line_style, colour = _split_border(value)
        out = [("border-%s-style" % side, line_style or "none")]
        if width is not None:
            out.append(("border-%s-width" % side, width))
        if colour is not None:
            out.append(("border-%s-color" % side, colour))
        return out

    if name == "background":
        # Seule la couleur est retenue : images et degrades ne sont pas peints.
        for token in split_shorthand(value):
            lowered = token.strip().lower()
            if lowered.startswith(("url(", "linear-gradient", "radial-gradient")):
                continue
            if lowered in ("none", "transparent"):
                return [("background-color", "transparent")]
            if csscolor.parse(token) is not None:
                return [("background-color", token)]
        return []

    if name == "font":
        return _expand_font(value)

    if name == "text-decoration":
        return [("text-decoration-line", value)]

    if name == "list-style":
        for token in split_shorthand(value.lower()):
            if token in ("disc", "circle", "square", "decimal", "none",
                         "lower-alpha", "upper-alpha", "lower-roman", "upper-roman"):
                return [("list-style-type", token)]
        return []

    return [(name, value)]


def _expand_font(value):
    """`font: italic bold 14px/1.5 sans-serif` en proprietes atomiques."""
    out = []
    for token in split_shorthand(value):
        lowered = token.strip().lower()
        if lowered in ("italic", "oblique"):
            out.append(("font-style", lowered))
        elif _is_bold(lowered):
            out.append(("font-weight", "bold"))
        elif "/" in lowered and lowered[0].isdigit():
            size_part, _, line_part = lowered.partition("/")
            out.append(("font-size", size_part))
            out.append(("line-height", line_part))
        elif lowered[:1].isdigit() or lowered in ABSOLUTE_FONT_SIZE_KEYWORDS:
            out.append(("font-size", lowered))
        elif lowered not in ("normal", "small-caps", "inherit"):
            out.append(("font-family", lowered))
    return out


ABSOLUTE_FONT_SIZE_KEYWORDS = frozenset(
    {"xx-small", "x-small", "small", "medium", "large", "x-large", "xx-large"}
)


def _is_bold(value):
    value = value.strip().lower()
    if value in ("bold", "bolder"):
        return True
    if value.isdigit():
        return int(value) >= 600
    return False


def _parse_line_height(value, font_size):
    value = value.strip().lower()
    if value == "normal":
        return int(font_size * 1.45)
    # Un nombre nu est un multiplicateur, pas une longueur.
    try:
        return max(1, int(round(float(value) * font_size)))
    except ValueError:
        pass
    pixels = to_pixels(parse_length(value), font_size, font_size)
    if pixels is None or pixels <= 0:
        return int(font_size * 1.45)
    return pixels


def _default_display(element):
    """Display initial avant cascade — la feuille UA le confirme ensuite."""
    if element.tag in ("head", "title", "meta", "link", "style", "script"):
        return "none"
    return "block" if element.is_block else "inline"


def _presentational_hints(element):
    """Attributs HTML de presentation traduits en declarations CSS."""
    hints = {}
    attrs = element.attrs
    tag = element.tag

    if "bgcolor" in attrs:
        hints["background-color"] = attrs["bgcolor"]
    if "color" in attrs and tag == "font":
        hints["color"] = attrs["color"]
    if "align" in attrs:
        value = attrs["align"].lower()
        if tag in ("img", "table"):
            if value in ("left", "right"):
                hints["float"] = value
        elif value in ("left", "right", "center", "justify"):
            hints["text-align"] = value
    if "width" in attrs and tag in ("img", "table", "td", "th", "canvas", "video", "iframe"):
        hints["width"] = _dimension(attrs["width"])
    if "height" in attrs and tag in ("img", "table", "td", "th", "canvas", "video", "iframe"):
        hints["height"] = _dimension(attrs["height"])
    if tag == "font" and "size" in attrs:
        hints["font-size"] = _font_size_attribute(attrs["size"])
    if tag == "hr" and "size" in attrs:
        hints["height"] = _dimension(attrs["size"])
    if tag in ("table", "td", "th") and "border" in attrs:
        width = attrs["border"] or "1"
        if width != "0":
            hints["border"] = "%spx solid #b7bcc2" % width
    if tag == "table" and "cellpadding" in attrs:
        hints["padding"] = _dimension(attrs["cellpadding"])
    if "hidden" in attrs:
        hints["display"] = "none"

    return hints


def _dimension(value):
    value = str(value).strip()
    if value.endswith("%"):
        return value
    digits = "".join(ch for ch in value if ch.isdigit() or ch == ".")
    return (digits + "px") if digits else "auto"


def _font_size_attribute(value):
    """Attribut `size` de `<font>` : echelle 1-7, ou relatif `+2`."""
    value = str(value).strip()
    table = {1: 10, 2: 13, 3: 16, 4: 18, 5: 24, 6: 32, 7: 48}
    try:
        if value.startswith(("+", "-")):
            level = 3 + int(value)
        else:
            level = int(value)
    except ValueError:
        return "16px"
    return "%dpx" % table.get(max(1, min(7, level)), 16)


def style_document(document, stylesheets=(), viewport=(1024, 768), context=None):
    """Raccourci : monte un moteur, ajoute la feuille UA puis les autres."""
    engine = StyleEngine(viewport=viewport, context=context)
    engine.add_stylesheet(user_agent_stylesheet())
    for sheet in stylesheets:
        if isinstance(sheet, str):
            engine.add_source(sheet)
        else:
            engine.add_stylesheet(sheet)
    engine.style_document(document)
    return engine


def extract_stylesheets(document, viewport=(1024, 768)):
    """Recupere les `<style>` du document. Renvoie (feuilles, liens externes)."""
    sheets = []
    links = []
    for node in document.descendants():
        if not isinstance(node, Element):
            continue
        if node.tag == "style":
            media = node.attrs.get("media", "")
            if media and not cssparser.evaluate_media(media, viewport):
                continue
            source = "".join(
                child.data for child in node.children if isinstance(child, Text)
            )
            if source.strip():
                sheets.append(cssparser.parse(source, viewport=viewport))
        elif node.tag == "link":
            rel = node.attrs.get("rel", "").lower()
            href = node.attrs.get("href", "")
            if "stylesheet" in rel and href:
                media = node.attrs.get("media", "")
                if media and not cssparser.evaluate_media(media, viewport):
                    continue
                links.append(href)
    return sheets, links
