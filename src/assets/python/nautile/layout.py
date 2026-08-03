"""Layout : de l'arbre style a la liste d'affichage.

Deux contextes de formatage, comme dans CSS 2.1 :

* **bloc** — les enfants s'empilent verticalement, les marges verticales
  adjacentes fusionnent, la largeur se resout contre le bloc conteneur ;
* **en ligne** — les fragments s'alignent horizontalement et passent a la
  ligne quand ils debordent.

Les elements positionnes (`absolute`, `fixed`) sortent du flux et vont dans
leur propre couche, ce qui leur donne un ordre d'empilement independant.
"""

from . import text as textmod
from .display_list import DisplayList, FormField, Image, Layer, Link, Rect, Text
from .dom import Element, Text as TextNode

# Elements de saisie rendus comme des boites atomiques.
_FIELD_TAGS = frozenset({"input", "textarea", "select", "button"})

# Hauteur d'une image sans dimension connue, avant chargement.
_DEFAULT_IMAGE_SIZE = (120, 90)

# Garde-fou : une page pathologique ne doit pas boucler indefiniment.
_MAX_DEPTH = 120


class _Target:
    """Destination des items : la liste principale ou une couche."""

    __slots__ = ("items", "links", "fields")

    def __init__(self, items, links, fields):
        self.items = items
        self.links = links
        self.fields = fields


class _InlineItem:
    """Fragment participant a une ligne."""

    __slots__ = ("kind", "text", "style", "href", "width", "height",
                 "element", "payload")

    def __init__(self, kind, style, width=0, height=0, text="", href=None,
                 element=None, payload=None):
        self.kind = kind
        self.style = style
        self.width = width
        self.height = height
        self.text = text
        self.href = href
        self.element = element
        self.payload = payload


class LayoutEngine:
    def __init__(self, width=1024, viewport=None, image_resolver=None):
        self.width = max(120, int(width))
        self.viewport = viewport if viewport else (self.width, 768)
        # Callback optionnel : (src, element) -> (largeur, hauteur, pixels).
        self.image_resolver = image_resolver

    # -- Entree ------------------------------------------------------------

    def layout(self, document):
        """Calcule la liste d'affichage complete d'un document style."""
        display_list = DisplayList(
            title=document.title,
            url=str(document.url) if document.url else "",
            width=self.width,
        )

        body = document.body
        root = document.document_element
        if body is None and root is None:
            return display_list

        display_list.background = self._page_background(root, body)

        container = body if body is not None else root
        if container is None or container.style is None:
            return display_list

        target = _Target(display_list.items, display_list.links, display_list.fields)
        bottom = self._layout_block(
            container, 0, 0, self.width, target, display_list, form=None, depth=0
        )
        display_list.height = max(bottom, 0)
        return display_list

    def _page_background(self, root, body):
        """Le fond de la page vient du body, sinon de html, sinon blanc."""
        for element in (body, root):
            if element is not None and element.style is not None:
                if element.style.background_color is not None:
                    return element.style.background_color
        return 0xFFFFFF

    # -- Contexte bloc -----------------------------------------------------

    def _layout_block(self, element, x, y, avail_width, target, display_list,
                      form, depth):
        """Place `element` a (x, y) dans `avail_width`. Renvoie le bas atteint."""
        style = element.style
        if style is None or style.is_none or depth > _MAX_DEPTH:
            return y

        margin_top, margin_right, margin_bottom, margin_left = style.margin
        border_top, border_right, border_bottom, border_left = style.border_width
        pad_top, pad_right, pad_bottom, pad_left = style.padding

        outer_x = x + margin_left
        horizontal_extra = (
            margin_left + margin_right + border_left + border_right + pad_left + pad_right
        )

        if style.width is not None:
            content_width = style.width
        else:
            content_width = avail_width - horizontal_extra
        content_width = max(0, content_width)
        if style.max_width is not None:
            content_width = min(content_width, style.max_width)
        if style.min_width is not None:
            content_width = max(content_width, style.min_width)

        # `margin: auto` centre le bloc dans son conteneur.
        if self._is_auto_margin(style):
            free = avail_width - (content_width + border_left + border_right + pad_left + pad_right)
            if free > 0:
                outer_x = x + free // 2

        border_box_x = outer_x
        content_x = border_box_x + border_left + pad_left
        content_y = y + border_top + pad_top

        # La position des enfants est connue, mais pas encore la hauteur : on
        # reserve la place du fond et des bordures pour les peindre apres.
        background_slot = len(target.items)

        inner_form = element if element.tag == "form" else form
        bottom = self._layout_children(
            element, content_x, content_y, content_width, target, display_list,
            inner_form, depth + 1
        )

        content_height = bottom - content_y
        if style.height is not None:
            content_height = style.height
        if style.min_height is not None:
            content_height = max(content_height, style.min_height)
        if style.max_height is not None:
            content_height = min(content_height, style.max_height)

        border_box_width = content_width + border_left + border_right + pad_left + pad_right
        border_box_height = content_height + border_top + border_bottom + pad_top + pad_bottom

        painted = self._paint_box(
            style, border_box_x, y, border_box_width, border_box_height
        )
        if painted:
            # Insere sous le contenu deja produit pour respecter l'empilement.
            target.items[background_slot:background_slot] = painted

        return y + border_box_height

    def _is_auto_margin(self, style):
        raw = style.raw
        return raw.get("margin-left", "").strip().lower() == "auto" and \
            raw.get("margin-right", "").strip().lower() == "auto"

    def _paint_box(self, style, x, y, width, height):
        """Fond et bordures d'une boite. Renvoie les items a inserer."""
        items = []
        if width <= 0 or height <= 0:
            return items
        if style.visibility == "hidden":
            return items

        if style.background_color is not None:
            items.append(Rect(x, y, width, height, style.background_color, style.border_radius))

        top, right, bottom, left = style.border_width
        colors = style.border_color
        fallback = style.color
        if top > 0:
            items.append(Rect(x, y, width, top, colors[0] if colors[0] is not None else fallback))
        if bottom > 0:
            items.append(
                Rect(x, y + height - bottom, width, bottom,
                     colors[2] if colors[2] is not None else fallback)
            )
        if left > 0:
            items.append(Rect(x, y, left, height, colors[3] if colors[3] is not None else fallback))
        if right > 0:
            items.append(
                Rect(x + width - right, y, right, height,
                     colors[1] if colors[1] is not None else fallback)
            )
        return items

    def _layout_children(self, element, x, y, width, target, display_list,
                         form, depth):
        """Enchaine les enfants : blocs empiles, suites en ligne regroupees."""
        cursor = y
        pending_margin = 0
        inline_run = []
        list_counter = [0]

        def flush_inline():
            nonlocal cursor, pending_margin
            if not inline_run:
                return
            items = []
            for node in inline_run:
                self._collect_inline(node, element.style, items, None, form, depth)
            inline_run.clear()
            if items:
                if pending_margin:
                    cursor += pending_margin
                    pending_margin = 0
                cursor = self._layout_line_box(
                    items, x, cursor, width, target, element.style
                )

        for child in element.children:
            if isinstance(child, TextNode):
                if child.data.strip() or (inline_run and child.data):
                    inline_run.append(child)
                continue
            if not isinstance(child, Element):
                continue

            style = child.style
            if style is None or style.is_none:
                continue

            if style.position in ("absolute", "fixed"):
                flush_inline()
                self._layout_positioned(child, x, cursor, width, display_list, form, depth)
                continue

            if style.is_block:
                flush_inline()
                if style.display == "list-item":
                    list_counter[0] += 1
                gap = max(pending_margin, style.margin[0])
                cursor += gap
                pending_margin = 0
                if style.display == "list-item":
                    self._paint_marker(child, x, cursor, target, list_counter[0])
                cursor = self._layout_block(
                    child, x, cursor, width, target, display_list, form, depth
                )
                pending_margin = style.margin[2]
            else:
                inline_run.append(child)

        flush_inline()
        cursor += pending_margin
        return cursor

    def _paint_marker(self, element, x, y, target, index):
        """Puce ou numero d'un `list-item`."""
        style = element.style
        kind = style.list_style_type
        if kind == "none":
            return
        metrics = textmod.metrics_for(style)
        if kind == "decimal":
            label = "%d." % index
        elif kind == "lower-alpha":
            label = "%s." % chr(ord("a") + (index - 1) % 26)
        elif kind == "upper-alpha":
            label = "%s." % chr(ord("A") + (index - 1) % 26)
        elif kind == "square":
            label = "▪"
        elif kind == "circle":
            label = "◦"
        else:
            label = "•"
        width = metrics.measure(label, style.font_size, style.bold)
        baseline = y + style.margin[0] + metrics.ascent(style.font_size)
        # Le marqueur se pose dans la marge gauche, hors de la boite de contenu.
        target.items.append(
            Text(x - width - 8, baseline, label, style.color, style.font_size,
                 style.bold, style.italic, style.monospace)
        )

    def _layout_positioned(self, element, x, y, width, display_list, form, depth):
        """Sort l'element du flux et lui donne sa propre couche."""
        style = element.style
        layer = Layer(z=style.z_index, fixed=(style.position == "fixed"))

        raw = style.raw
        left = _pixels(raw.get("left"), style, self.viewport, width)
        top = _pixels(raw.get("top"), style, self.viewport, self.viewport[1])
        right = _pixels(raw.get("right"), style, self.viewport, width)

        origin_x = x if left is None else (x + left)
        if left is None and right is not None:
            box_width = style.width if style.width is not None else width // 3
            origin_x = x + width - right - box_width
        origin_y = y if top is None else top

        if style.overflow in ("hidden", "auto", "scroll") and style.width and style.height:
            layer.clip = (origin_x, origin_y, style.width, style.height)

        target = _Target(layer.items, layer.links, layer.fields)
        self._layout_block(
            element, origin_x, origin_y, width, target, display_list, form, depth + 1
        )
        if layer.items or layer.links or layer.fields:
            display_list.layers.append(layer)

    # -- Contexte en ligne -------------------------------------------------

    def _collect_inline(self, node, parent_style, out, href, form, depth):
        """Aplati un sous-arbre en ligne en fragments placables."""
        if depth > _MAX_DEPTH:
            return

        if isinstance(node, TextNode):
            style = parent_style
            if style is None:
                return
            content = node.data
            if style.white_space in ("pre", "pre-wrap"):
                segments = content.split("\n")
                for index, segment in enumerate(segments):
                    if index:
                        out.append(_InlineItem("break", style))
                    if segment:
                        self._push_words(segment, style, out, href)
            else:
                normalized = textmod.normalize_whitespace(content)
                if normalized.strip() or (out and normalized):
                    self._push_words(normalized, style, out, href)
            return

        if not isinstance(node, Element):
            return

        style = node.style
        if style is None or style.is_none:
            return

        tag = node.tag

        if tag == "br":
            out.append(_InlineItem("break", style))
            return

        if tag == "img":
            out.append(self._make_image_item(node, style, href))
            return

        if tag in _FIELD_TAGS:
            item = self._make_field_item(node, style, form)
            if item is not None:
                out.append(item)
            return

        if style.display in ("inline-block", "inline-flex") or style.is_block:
            # Boite atomique : mise en page a part, inseree comme un bloc-mot.
            item = self._make_atomic_item(node, style, href, form, depth)
            if item is not None:
                out.append(item)
            return

        child_href = node.attrs.get("href") if tag == "a" else href
        for child in node.children:
            self._collect_inline(child, style, out, child_href, form, depth + 1)

    def _push_words(self, content, style, out, href):
        metrics = textmod.metrics_for(style)
        for word in textmod.split_words(content):
            if not word:
                continue
            out.append(
                _InlineItem(
                    "text",
                    style,
                    width=metrics.measure(word, style.font_size, style.bold),
                    height=style.line_height,
                    text=word,
                    href=href,
                )
            )

    def _make_image_item(self, element, style, href):
        src = element.attrs.get("src", "")
        alt = element.attrs.get("alt", "")
        width = style.width
        height = style.height
        data = None
        if self.image_resolver is not None:
            resolved = self.image_resolver(src, element)
            if resolved is not None:
                natural_width, natural_height, data = resolved
                if width is None and height is None:
                    width, height = natural_width, natural_height
                elif width is None and natural_height:
                    width = max(1, natural_width * height // natural_height)
                elif height is None and natural_width:
                    height = max(1, natural_height * width // natural_width)
        if width is None:
            width = _DEFAULT_IMAGE_SIZE[0] if not alt else min(
                400, textmod.DEFAULT.measure(alt, style.font_size) + 12
            )
        if height is None:
            height = _DEFAULT_IMAGE_SIZE[1] if not alt else style.line_height
        return _InlineItem(
            "image", style, width=width, height=height, href=href,
            element=element, payload=(src, alt, data),
        )

    def _make_field_item(self, element, style, form):
        kind = (element.attrs.get("type") or "text").lower()
        if element.tag == "button":
            kind = "submit"
        elif element.tag == "textarea":
            kind = "textarea"
        elif element.tag == "select":
            kind = "select"
        if kind == "hidden":
            return None

        if element.tag == "textarea":
            value = element.text_content
        else:
            value = element.attrs.get("value", "")

        label = value or element.attrs.get("placeholder", "")
        metrics = textmod.metrics_for(style)
        if style.width is not None:
            width = style.width
        elif kind in ("submit", "button", "reset"):
            width = metrics.measure(label or "OK", style.font_size, style.bold) + 24
        else:
            size_attr = element.attrs.get("size")
            characters = int(size_attr) if (size_attr or "").isdigit() else 20
            width = metrics.measure("0" * characters, style.font_size) + 12
        height = style.height if style.height is not None else style.line_height + 10

        action = form.attrs.get("action", "") if form is not None else ""
        method = (form.attrs.get("method", "get") if form is not None else "get").lower()
        hidden = []
        if form is not None:
            for node in form.descendants():
                if (
                    isinstance(node, Element)
                    and node.tag == "input"
                    and (node.attrs.get("type") or "").lower() == "hidden"
                ):
                    name = node.attrs.get("name")
                    if name:
                        hidden.append((name, node.attrs.get("value", "")))

        return _InlineItem(
            "field", style, width=width, height=height, element=element,
            payload=(kind, element.attrs.get("name", ""), value,
                     element.attrs.get("placeholder", ""), action, method, hidden),
        )

    def _make_atomic_item(self, element, style, href, form, depth):
        """Met en page une boite inline-block dans un espace local."""
        preferred = self._preferred_width(element)
        available = max(40, self.width)
        box_width = style.width if style.width is not None else min(preferred, available)

        local = DisplayList(width=box_width)
        target = _Target(local.items, local.links, local.fields)
        bottom = self._layout_block(
            element, 0, 0, box_width + _horizontal_extra(style), target,
            local, form, depth + 1
        )
        width = box_width + _horizontal_extra(style)
        return _InlineItem(
            "atomic", style, width=width, height=bottom, href=href,
            element=element, payload=(local.items, local.links, local.fields),
        )

    def _preferred_width(self, element):
        """Largeur max-content approximative, pour dimensionner une boite atomique."""
        style = element.style
        if style is None or style.is_none:
            return 0
        if style.width is not None:
            return style.width + _horizontal_extra(style)

        total = 0
        for node in element.descendants():
            if isinstance(node, TextNode):
                owner = node.parent_node
                owner_style = owner.style if isinstance(owner, Element) else style
                if owner_style is None:
                    continue
                metrics = textmod.metrics_for(owner_style)
                total += metrics.measure(
                    textmod.normalize_whitespace(node.data),
                    owner_style.font_size,
                    owner_style.bold,
                )
            elif isinstance(node, Element) and node.tag == "img":
                total += node.style.width if node.style and node.style.width else _DEFAULT_IMAGE_SIZE[0]
        return total + _horizontal_extra(style)

    def _layout_line_box(self, items, x, y, width, target, parent_style):
        """Repartit les fragments en lignes et les peint. Renvoie le bas."""
        if not items:
            return y

        align = parent_style.text_align if parent_style else "left"
        cursor_y = y
        line = []
        line_width = 0
        indent = parent_style.text_indent if parent_style else 0
        available = max(1, width - indent)

        def commit(force_height=None):
            nonlocal cursor_y, line, line_width, indent, available
            if not line:
                if force_height:
                    cursor_y += force_height
                return
            height = max(item.height for item in line)
            offset = 0
            if align in ("center", "right"):
                slack = available - line_width
                if slack > 0:
                    offset = slack // 2 if align == "center" else slack
            self._paint_line(line, x + indent + offset, cursor_y, height, target)
            cursor_y += height
            line = []
            line_width = 0
            indent = 0
            available = max(1, width)

        for item in items:
            if item.kind == "break":
                commit(force_height=item.style.line_height if not line else None)
                continue

            if line_width + item.width > available and line:
                commit()

            if item.width > available and item.kind == "text":
                # Mot plus large que la ligne : on le coupe pour rester dans le cadre.
                metrics = textmod.metrics_for(item.style)
                remaining = item.text
                while remaining:
                    head, remaining = textmod.break_long_word(
                        remaining, metrics, item.style.font_size, item.style.bold,
                        available - line_width,
                    )
                    if not head:
                        commit()
                        continue
                    piece = _InlineItem(
                        "text", item.style,
                        width=metrics.measure(head, item.style.font_size, item.style.bold),
                        height=item.height, text=head, href=item.href,
                    )
                    line.append(piece)
                    line_width += piece.width
                    if remaining:
                        commit()
                continue

            line.append(item)
            line_width += item.width

        commit()
        return cursor_y

    def _paint_line(self, line, x, y, height, target):
        cursor = x
        for item in line:
            style = item.style

            if item.kind == "text":
                if style.visibility != "hidden" and item.text.strip():
                    metrics = textmod.metrics_for(style)
                    baseline = y + (height - style.font_size) // 2 + metrics.ascent(style.font_size)
                    if style.background_color is not None:
                        target.items.append(
                            Rect(cursor, y, item.width, height, style.background_color)
                        )
                    target.items.append(
                        Text(cursor, baseline, item.text, style.color, style.font_size,
                             style.bold, style.italic, style.monospace,
                             style.underline, style.line_through)
                    )
                if item.href:
                    target.links.append(
                        Link(cursor, y, item.width, height, item.href)
                    )

            elif item.kind == "image":
                src, alt, data = item.payload
                target.items.append(
                    Image(cursor, y, item.width, item.height, src, alt, data)
                )
                if item.href:
                    target.links.append(Link(cursor, y, item.width, item.height, item.href))

            elif item.kind == "field":
                kind, name, value, placeholder, action, method, hidden = item.payload
                painted = self._paint_box(style, cursor, y, item.width, item.height)
                target.items.extend(painted)
                label = value or placeholder
                if label:
                    metrics = textmod.metrics_for(style)
                    baseline = y + (item.height - style.font_size) // 2 + metrics.ascent(style.font_size)
                    shade = style.color if value else 0x8A8F98
                    target.items.append(
                        Text(cursor + 6, baseline, label, shade, style.font_size,
                             style.bold, style.italic, style.monospace)
                    )
                target.fields.append(
                    FormField(cursor, y, item.width, item.height, name, value,
                              kind, action, method, hidden, placeholder)
                )

            elif item.kind == "atomic":
                items, links, fields = item.payload
                for sub in items:
                    target.items.append(_translate(sub, cursor, y))
                for link in links:
                    target.links.append(
                        Link(link.x + cursor, link.y + y, link.w, link.h, link.href)
                    )
                for field in fields:
                    field.x += cursor
                    field.y += y
                    target.fields.append(field)
                if item.href:
                    target.links.append(Link(cursor, y, item.width, item.height, item.href))

            cursor += item.width


def _horizontal_extra(style):
    return (
        style.margin[1] + style.margin[3]
        + style.border_width[1] + style.border_width[3]
        + style.padding[1] + style.padding[3]
    )


def _pixels(raw, style, viewport, percent_base):
    from .css.values import parse_length, to_pixels

    if raw is None:
        return None
    if raw.strip().lower() == "auto":
        return None
    return to_pixels(parse_length(raw), style.font_size, percent_base, viewport)


def _translate(item, dx, dy):
    """Copie un item decale — les boites atomiques sont posees en coords locales."""
    if isinstance(item, Rect):
        return Rect(item.x + dx, item.y + dy, item.w, item.h, item.color, item.radius)
    if isinstance(item, Text):
        return Text(item.x + dx, item.y + dy, item.text, item.color, item.size,
                    item.bold, item.italic, item.monospace, item.underline,
                    item.line_through)
    if isinstance(item, Image):
        return Image(item.x + dx, item.y + dy, item.w, item.h, item.src,
                     item.alt, item.data)
    return item


def layout(document, width=1024, viewport=None, image_resolver=None):
    """Raccourci : construit un moteur et met en page un document style."""
    engine = LayoutEngine(width=width, viewport=viewport, image_resolver=image_resolver)
    return engine.layout(document)
