"""Contrat commun aux backends de rendu.

Un backend consomme une `DisplayList` et sait dessiner trois primitives :
rectangle, texte, image. Le reste — defilement, empilement des couches,
clipping — est fait ici, une fois pour toutes, pour que chaque backend reste
minimal.
"""

from ..display_list import Image, Rect, Text


class Canvas:
    """Surface de dessin. Les backends implementent les trois primitives."""

    def fill_rect(self, x, y, w, h, color, radius=0):
        raise NotImplementedError

    def draw_text(self, x, y, text, color, size, bold=False, italic=False,
                  monospace=False, underline=False, line_through=False):
        raise NotImplementedError

    def draw_image(self, x, y, w, h, image):
        """Par defaut : cadre et texte alternatif, ce que fait un navigateur
        quand l'image ne charge pas."""
        self.fill_rect(x, y, w, h, 0xEEF0F3)
        self.fill_rect(x, y, w, 1, 0xC7CCD1)
        self.fill_rect(x, y + h - 1, w, 1, 0xC7CCD1)
        self.fill_rect(x, y, 1, h, 0xC7CCD1)
        self.fill_rect(x + w - 1, y, 1, h, 0xC7CCD1)
        if image.alt and w > 24 and h > 12:
            self.draw_text(x + 4, y + min(h - 4, 14), image.alt, 0x5F6368, 12)

    def clip_push(self, x, y, w, h):
        """Restreint le dessin. Les backends sans clipping l'ignorent."""
        return None

    def clip_pop(self):
        return None


def render(canvas, display, scroll=0, viewport=None, origin=(0, 0)):
    """Peint une `DisplayList` sur un canvas.

    `scroll` est le decalage vertical courant, `viewport` la zone visible
    (largeur, hauteur) pour ne dessiner que ce qui compte, et `origin` le
    coin haut-gauche de la zone de contenu dans la surface du backend.
    """
    origin_x, origin_y = origin
    view_width = viewport[0] if viewport else display.width
    view_height = viewport[1] if viewport else display.height

    canvas.fill_rect(origin_x, origin_y, view_width, view_height, display.background)

    layers = display.sorted_layers()
    # Sous le flux normal : couches d'index negatif.
    for layer in layers:
        if layer.z < 0:
            _paint_layer(canvas, layer, scroll, origin, (view_width, view_height))

    _paint_items(canvas, display.items, scroll, origin, (view_width, view_height))

    for layer in layers:
        if layer.z >= 0:
            _paint_layer(canvas, layer, scroll, origin, (view_width, view_height))


def _paint_layer(canvas, layer, scroll, origin, viewport):
    offset = 0 if layer.fixed else scroll
    clipped = False
    if layer.clip is not None:
        cx, cy, cw, ch = layer.clip
        canvas.clip_push(origin[0] + cx, origin[1] + cy - offset, cw, ch)
        clipped = True
    _paint_items(canvas, layer.items, offset, origin, viewport)
    if clipped:
        canvas.clip_pop()


def _paint_items(canvas, items, scroll, origin, viewport):
    origin_x, origin_y = origin
    view_width, view_height = viewport

    for item in items:
        y = item.y - scroll

        if isinstance(item, Rect):
            # Hors champ : on saute sans appeler le backend.
            if y + item.h < 0 or y > view_height:
                continue
            canvas.fill_rect(
                origin_x + item.x, origin_y + y, item.w, item.h, item.color, item.radius
            )

        elif isinstance(item, Text):
            # La coordonnee d'un texte est sa ligne de base.
            if y + item.size < 0 or y - item.size > view_height:
                continue
            canvas.draw_text(
                origin_x + item.x, origin_y + y, item.text, item.color, item.size,
                item.bold, item.italic, item.monospace, item.underline,
                item.line_through,
            )

        elif isinstance(item, Image):
            if y + item.h < 0 or y > view_height:
                continue
            canvas.draw_image(origin_x + item.x, origin_y + y, item.w, item.h, item)
