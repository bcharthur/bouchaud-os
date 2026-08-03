"""Liste d'affichage.

Produit du layout, consommee par les backends de rendu. C'est la frontiere
stable du moteur : tout ce qui sait dessiner un rectangle, du texte et une
image peut afficher une page Nautile — le framebuffer de Bouchaud OS, un
canvas Tk, un terminal.

Les coordonnees sont en pixels documents (origine en haut a gauche de la
page, avant defilement). C'est au backend d'appliquer le scroll.
"""


class Rect:
    __slots__ = ("x", "y", "w", "h", "color", "radius")

    def __init__(self, x, y, w, h, color, radius=0):
        self.x = x
        self.y = y
        self.w = w
        self.h = h
        self.color = color
        self.radius = radius

    def __repr__(self):
        return "Rect(%d,%d %dx%d #%06x)" % (self.x, self.y, self.w, self.h, self.color)


class Text:
    __slots__ = ("x", "y", "text", "color", "size", "bold", "italic",
                 "monospace", "underline", "line_through")

    def __init__(self, x, y, text, color, size=16, bold=False, italic=False,
                 monospace=False, underline=False, line_through=False):
        self.x = x
        self.y = y
        self.text = text
        self.color = color
        self.size = size
        self.bold = bold
        self.italic = italic
        self.monospace = monospace
        self.underline = underline
        self.line_through = line_through

    def __repr__(self):
        preview = self.text if len(self.text) <= 20 else self.text[:17] + "..."
        return "Text(%d,%d %r)" % (self.x, self.y, preview)


class Image:
    __slots__ = ("x", "y", "w", "h", "src", "alt", "data")

    def __init__(self, x, y, w, h, src="", alt="", data=None):
        self.x = x
        self.y = y
        self.w = w
        self.h = h
        self.src = src
        self.alt = alt
        # Pixels decodes, remplis par le chargeur quand le backend sait afficher.
        self.data = data

    def __repr__(self):
        return "Image(%d,%d %dx%d %r)" % (self.x, self.y, self.w, self.h, self.src)


class Link:
    """Zone cliquable menant a `href`."""

    __slots__ = ("x", "y", "w", "h", "href", "title")

    def __init__(self, x, y, w, h, href, title=""):
        self.x = x
        self.y = y
        self.w = w
        self.h = h
        self.href = href
        self.title = title

    def contains(self, px, py):
        return self.x <= px < self.x + self.w and self.y <= py < self.y + self.h

    def __repr__(self):
        return "Link(%d,%d %dx%d -> %s)" % (self.x, self.y, self.w, self.h, self.href)


class FormField:
    """Champ de saisie interactif, aplati avec le contexte de son `<form>`."""

    __slots__ = ("x", "y", "w", "h", "name", "value", "kind", "action",
                 "method", "hidden", "placeholder")

    def __init__(self, x, y, w, h, name="", value="", kind="text", action="",
                 method="get", hidden=None, placeholder=""):
        self.x = x
        self.y = y
        self.w = w
        self.h = h
        self.name = name
        self.value = value
        self.kind = kind
        self.action = action
        self.method = method
        # Champs caches du meme formulaire, necessaires pour construire l'URL.
        self.hidden = hidden if hidden is not None else []
        self.placeholder = placeholder

    def contains(self, px, py):
        return self.x <= px < self.x + self.w and self.y <= py < self.y + self.h

    def __repr__(self):
        return "FormField(%r=%r)" % (self.name, self.value)


class Layer:
    """Couche d'empilement : items partageant un ordre `z`.

    `fixed` ancre la couche au viewport (elle ignore le defilement) et `clip`
    restreint son dessin a un rectangle — c'est ce qui rend `position: fixed`
    et `overflow: hidden` observables.
    """

    __slots__ = ("z", "fixed", "clip", "items", "links", "fields")

    def __init__(self, z=0, fixed=False, clip=None):
        self.z = z
        self.fixed = fixed
        self.clip = clip
        self.items = []
        self.links = []
        self.fields = []

    def __repr__(self):
        return "Layer(z=%d, %d items%s)" % (
            self.z, len(self.items), ", fixed" if self.fixed else ""
        )


class DisplayList:
    """Resultat complet du layout d'un document."""

    __slots__ = ("title", "url", "items", "links", "fields", "layers",
                 "width", "height", "background")

    def __init__(self, title="", url="", width=0, background=0xFFFFFF):
        self.title = title
        self.url = url
        self.items = []
        self.links = []
        self.fields = []
        self.layers = []
        self.width = width
        self.height = 0
        self.background = background

    def add(self, item):
        self.items.append(item)
        return item

    def sorted_layers(self):
        return sorted(self.layers, key=lambda layer: layer.z)

    def all_links(self):
        found = list(self.links)
        for layer in self.layers:
            found.extend(layer.links)
        return found

    def all_fields(self):
        found = list(self.fields)
        for layer in self.layers:
            found.extend(layer.fields)
        return found

    def link_at(self, x, y):
        """Lien sous le point donne. Les couches hautes gagnent."""
        for layer in sorted(self.layers, key=lambda l: l.z, reverse=True):
            for link in reversed(layer.links):
                if link.contains(x, y):
                    return link
        for link in reversed(self.links):
            if link.contains(x, y):
                return link
        return None

    def field_at(self, x, y):
        for layer in sorted(self.layers, key=lambda l: l.z, reverse=True):
            for field in reversed(layer.fields):
                if field.contains(x, y):
                    return field
        for field in reversed(self.fields):
            if field.contains(x, y):
                return field
        return None

    def text_content(self):
        """Texte de la page dans l'ordre de peinture — pour `--dump` et tests."""
        # Les fragments portent deja leur espace de fin : les joindre avec un
        # separateur doublerait les blancs.
        parts = []
        for item in self.items:
            if isinstance(item, Text):
                parts.append(item.text)
        return "".join(parts)

    def __repr__(self):
        return "DisplayList(%r, %d items, %dx%d)" % (
            self.title, len(self.items), self.width, self.height
        )
