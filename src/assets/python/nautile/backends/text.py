"""Backend texte : rend une page dans un terminal.

Utile pour deux choses : le mode `--dump` en ligne de commande, et les tests
automatises, qui verifient ainsi la chaine complete jusqu'a la peinture sans
dependre d'un serveur graphique.
"""

from ..display_list import Image, Rect, Text
from .base import Canvas


class TextCanvas(Canvas):
    """Grille de caracteres. Le texte y est pose a sa position proportionnelle."""

    def __init__(self, columns=100, rows=60, cell=(8, 16)):
        self.columns = columns
        self.rows = rows
        self.cell_width, self.cell_height = cell
        self.grid = [[" "] * columns for _ in range(rows)]
        # Colonne libre suivante par ligne : une police proportionnelle est
        # plus etroite qu'une cellule, donc deux fragments voisins tomberaient
        # sur les memes colonnes et l'un ecraserait l'autre.
        self.next_free = [0] * rows

    def _cell(self, x, y):
        return int(x // self.cell_width), int(y // self.cell_height)

    def fill_rect(self, x, y, w, h, color, radius=0):
        # Les aplats ne sont pas rendus : ils masqueraient le texte.
        return

    def draw_text(self, x, y, text, color, size, bold=False, italic=False,
                  monospace=False, underline=False, line_through=False):
        if not text.strip():
            return
        column, row = self._cell(x, y - size)
        if not (0 <= row < self.rows):
            return

        # On respecte l'indentation demandee, mais jamais au prix d'un
        # chevauchement : le fragment glisse a droite de ce qui est deja pose.
        floor = self.next_free[row]
        if floor > 0:
            floor += 1
        column = max(column, floor, 0)
        if column >= self.columns:
            return

        content = text.rstrip()
        if not content:
            return
        line = self.grid[row]
        written = 0
        for offset, char in enumerate(content):
            position = column + offset
            if position >= self.columns:
                break
            line[position] = char
            written = offset + 1
        self.next_free[row] = column + written

    def draw_image(self, x, y, w, h, image):
        label = "[%s]" % (image.alt or "image")
        self.draw_text(x, y + 12, label, 0, 12)

    def to_text(self):
        lines = ["".join(row).rstrip() for row in self.grid]
        while lines and not lines[-1]:
            lines.pop()
        return "\n".join(lines)


def render_page(display, columns=100, max_rows=400, scroll=0):
    """Rend une liste d'affichage en texte."""
    rows = min(max_rows, max(4, (display.height // 16) + 2))
    canvas = TextCanvas(columns=columns, rows=rows)
    from .base import render

    render(canvas, display, scroll=scroll, viewport=(display.width, display.height))
    return canvas.to_text()


def dump(page, columns=100, show_links=True):
    """Rendu texte complet d'une page, avec ses liens — le mode `--dump`."""
    display = page.display_list
    out = []
    out.append("=" * columns)
    out.append(page.title or "(sans titre)")
    out.append(page.url)
    out.append("=" * columns)
    out.append(render_page(display, columns=columns))

    if show_links:
        links = display.all_links()
        if links:
            out.append("")
            out.append("-- liens " + "-" * (columns - 9))
            seen = []
            for link in links:
                if link.href not in seen:
                    seen.append(link.href)
            for index, href in enumerate(seen[:40], 1):
                out.append("[%2d] %s" % (index, href))

        fields = display.all_fields()
        if fields:
            out.append("")
            out.append("-- champs " + "-" * (columns - 10))
            for field in fields:
                out.append(
                    "  %-16s %-10s %s"
                    % (field.name or "(sans nom)", field.kind, field.value or field.placeholder)
                )
    return "\n".join(out)


def outline(display):
    """Structure de la page : un item par ligne. Pratique en debug."""
    lines = []
    for item in display.items:
        if isinstance(item, Text):
            lines.append("text  %4d,%4d  %s" % (item.x, item.y, item.text))
        elif isinstance(item, Rect):
            lines.append("rect  %4d,%4d  %dx%d #%06x" % (item.x, item.y, item.w, item.h, item.color))
        elif isinstance(item, Image):
            lines.append("image %4d,%4d  %dx%d %s" % (item.x, item.y, item.w, item.h, item.src))
    return "\n".join(lines)
