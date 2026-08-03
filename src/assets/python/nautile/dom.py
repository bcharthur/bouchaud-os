"""Arbre DOM.

Modele minimal mais fidele : `Node` generique, `Element` avec attributs et
selecteurs, `Text`, `Comment`, `Document`. Les API exposees suivent le DOM
standard (`children`, `parent_node`, `get_element_by_id`, `query_selector`)
pour que le style et le layout se lisent comme dans un vrai moteur.
"""

# Elements dont le contenu est du texte brut : le tokenizer ne cherche pas de
# balises a l'interieur, seulement la balise fermante correspondante.
RAW_TEXT_ELEMENTS = frozenset({"script", "style", "textarea", "title", "xmp"})

# Elements sans contenu ni balise fermante.
VOID_ELEMENTS = frozenset(
    {
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link",
        "meta", "param", "source", "track", "wbr",
    }
)

# Elements de niveau bloc pour le layout par defaut (feuille user-agent).
BLOCK_ELEMENTS = frozenset(
    {
        "address", "article", "aside", "blockquote", "body", "center", "dd",
        "details", "div", "dl", "dt", "fieldset", "figcaption", "figure",
        "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hr",
        "html", "li", "main", "nav", "ol", "p", "pre", "section", "summary",
        "table", "tbody", "td", "tfoot", "th", "thead", "tr", "ul",
    }
)


class Node:
    """Noeud generique. `children` est toujours une liste (vide si feuille)."""

    __slots__ = ("parent_node", "children", "document")

    def __init__(self):
        self.parent_node = None
        self.children = []
        self.document = None

    # -- Arbre -------------------------------------------------------------

    def append_child(self, node):
        if node.parent_node is not None:
            node.parent_node.remove_child(node)
        node.parent_node = self
        node.document = self.document
        self.children.append(node)
        return node

    def insert_before(self, node, reference):
        if reference is None:
            return self.append_child(node)
        idx = self.children.index(reference)
        if node.parent_node is not None:
            node.parent_node.remove_child(node)
        node.parent_node = self
        node.document = self.document
        self.children.insert(idx, node)
        return node

    def remove_child(self, node):
        self.children.remove(node)
        node.parent_node = None
        return node

    @property
    def first_child(self):
        return self.children[0] if self.children else None

    @property
    def last_child(self):
        return self.children[-1] if self.children else None

    @property
    def next_sibling(self):
        parent = self.parent_node
        if parent is None:
            return None
        idx = parent.children.index(self)
        if idx + 1 < len(parent.children):
            return parent.children[idx + 1]
        return None

    @property
    def previous_sibling(self):
        parent = self.parent_node
        if parent is None:
            return None
        idx = parent.children.index(self)
        return parent.children[idx - 1] if idx > 0 else None

    def descendants(self):
        """Parcours prefixe de tout le sous-arbre, self exclu."""
        stack = list(reversed(self.children))
        while stack:
            node = stack.pop()
            yield node
            stack.extend(reversed(node.children))

    def ancestors(self):
        node = self.parent_node
        while node is not None:
            yield node
            node = node.parent_node

    # -- Contenu -----------------------------------------------------------

    @property
    def text_content(self):
        parts = []
        for node in self.descendants():
            if isinstance(node, Text):
                parts.append(node.data)
        return "".join(parts)


class Text(Node):
    __slots__ = ("data",)

    def __init__(self, data=""):
        Node.__init__(self)
        self.data = data

    @property
    def text_content(self):
        return self.data

    def __repr__(self):
        preview = self.data if len(self.data) <= 30 else self.data[:27] + "..."
        return "Text(%r)" % preview


class Comment(Node):
    __slots__ = ("data",)

    def __init__(self, data=""):
        Node.__init__(self)
        self.data = data

    @property
    def text_content(self):
        return ""

    def __repr__(self):
        return "Comment(%r)" % self.data[:30]


class Element(Node):
    __slots__ = ("tag", "attrs", "style", "layout_box")

    def __init__(self, tag, attrs=None):
        Node.__init__(self)
        self.tag = tag.lower()
        self.attrs = dict(attrs) if attrs else {}
        # Rempli par le module `style` : dict des proprietes calculees.
        self.style = None
        self.layout_box = None

    # -- Attributs ---------------------------------------------------------

    def get(self, name, default=None):
        return self.attrs.get(name.lower(), default)

    def set(self, name, value):
        self.attrs[name.lower()] = value

    def has(self, name):
        return name.lower() in self.attrs

    @property
    def id(self):
        return self.attrs.get("id")

    @property
    def class_list(self):
        value = self.attrs.get("class")
        return value.split() if value else []

    @property
    def is_block(self):
        return self.tag in BLOCK_ELEMENTS

    # -- Recherche ---------------------------------------------------------

    def get_element_by_id(self, element_id):
        for node in self.descendants():
            if isinstance(node, Element) and node.attrs.get("id") == element_id:
                return node
        return None

    def get_elements_by_tag(self, tag):
        tag = tag.lower()
        return [
            n
            for n in self.descendants()
            if isinstance(n, Element) and (tag == "*" or n.tag == tag)
        ]

    def get_elements_by_class(self, name):
        return [
            n
            for n in self.descendants()
            if isinstance(n, Element) and name in n.class_list
        ]

    def query_selector(self, selector):
        """Selecteur simple : `tag`, `#id`, `.classe` ou une combinaison."""
        for node in self.descendants():
            if isinstance(node, Element) and _matches_simple(node, selector):
                return node
        return None

    def query_selector_all(self, selector):
        return [
            n
            for n in self.descendants()
            if isinstance(n, Element) and _matches_simple(n, selector)
        ]

    def __repr__(self):
        bits = [self.tag]
        if self.attrs.get("id"):
            bits.append("#" + self.attrs["id"])
        for cls in self.class_list[:2]:
            bits.append("." + cls)
        return "<%s>" % "".join(bits)


class Document(Node):
    __slots__ = ("url", "doctype", "_root")

    def __init__(self, url=None):
        Node.__init__(self)
        self.document = self
        self.url = url
        self.doctype = None
        self._root = None

    @property
    def document_element(self):
        """Element `<html>`, ou le premier element trouve."""
        if self._root is not None:
            return self._root
        for child in self.children:
            if isinstance(child, Element):
                self._root = child
                return child
        return None

    @property
    def head(self):
        root = self.document_element
        if root is None:
            return None
        for child in root.children:
            if isinstance(child, Element) and child.tag == "head":
                return child
        return None

    @property
    def body(self):
        root = self.document_element
        if root is None:
            return None
        for child in root.children:
            if isinstance(child, Element) and child.tag == "body":
                return child
        return None

    @property
    def title(self):
        head = self.head
        scope = head if head is not None else self
        for node in scope.descendants():
            if isinstance(node, Element) and node.tag == "title":
                return node.text_content.strip()
        return ""

    # Les recherches d'un Document delegent a son element racine.
    def get_element_by_id(self, element_id):
        for node in self.descendants():
            if isinstance(node, Element) and node.attrs.get("id") == element_id:
                return node
        return None

    def get_elements_by_tag(self, tag):
        tag = tag.lower()
        return [
            n
            for n in self.descendants()
            if isinstance(n, Element) and (tag == "*" or n.tag == tag)
        ]

    def query_selector(self, selector):
        for node in self.descendants():
            if isinstance(node, Element) and _matches_simple(node, selector):
                return node
        return None

    def query_selector_all(self, selector):
        return [
            n
            for n in self.descendants()
            if isinstance(n, Element) and _matches_simple(n, selector)
        ]

    def __repr__(self):
        return "Document(%r)" % (self.url,)


def _matches_simple(element, selector):
    """Teste un selecteur compose sans combinateur : `div#a.b.c`."""
    selector = selector.strip()
    if not selector or selector == "*":
        return True
    tag = ""
    ids = []
    classes = []
    buf = ""
    kind = "tag"
    for ch in selector:
        if ch in "#.":
            if kind == "tag":
                tag = buf
            elif kind == "#":
                ids.append(buf)
            else:
                classes.append(buf)
            buf = ""
            kind = ch
        else:
            buf += ch
    if kind == "tag":
        tag = buf
    elif kind == "#":
        ids.append(buf)
    elif buf:
        classes.append(buf)

    if tag and tag != "*" and element.tag != tag.lower():
        return False
    for wanted in ids:
        if element.attrs.get("id") != wanted:
            return False
    if classes:
        present = element.class_list
        for wanted in classes:
            if wanted not in present:
                return False
    return True


def dump_tree(node, indent=0, out=None):
    """Rend l'arbre en texte indente — utile en debug et dans les tests."""
    if out is None:
        out = []
    prefix = "  " * indent
    if isinstance(node, Document):
        out.append(prefix + "#document")
    elif isinstance(node, Element):
        attrs = "".join(' %s="%s"' % kv for kv in sorted(node.attrs.items()))
        out.append("%s<%s%s>" % (prefix, node.tag, attrs))
    elif isinstance(node, Text):
        stripped = node.data.strip()
        if not stripped:
            return out
        out.append('%s"%s"' % (prefix, stripped))
    elif isinstance(node, Comment):
        out.append("%s<!--%s-->" % (prefix, node.data.strip()))
    for child in node.children:
        dump_tree(child, indent + 1, out)
    return out
