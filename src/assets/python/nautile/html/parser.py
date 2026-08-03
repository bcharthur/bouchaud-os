"""Construction de l'arbre DOM a partir des jetons.

Le standard decrit une machine a 23 modes d'insertion. On implemente le
comportement observable qui compte pour le rendu : creation implicite de
`<html>`/`<head>/<body>`, fermeture implicite des elements ouverts (`<p>`,
`<li>`, `<tr>`, `<td>`, `<option>`), correction des balises fermantes
orphelines, et confinement du contenu de tableau.
"""

from ..dom import Comment, Document, Element, Text, VOID_ELEMENTS
from . import tokenizer
from .tokenizer import COMMENT, DOCTYPE, END_TAG, START_TAG, TEXT

# Elements qui ferment implicitement un `<p>` ouvert quand ils s'ouvrent.
CLOSES_PARAGRAPH = frozenset(
    {
        "address", "article", "aside", "blockquote", "details", "div", "dl",
        "fieldset", "figcaption", "figure", "footer", "form", "h1", "h2", "h3",
        "h4", "h5", "h6", "header", "hr", "main", "nav", "ol", "p", "pre",
        "section", "table", "ul",
    }
)

# Fermetures implicites entre pairs : ouvrir la cle ferme les valeurs ouvertes.
IMPLIED_END = {
    "li": ("li",),
    "dt": ("dt", "dd"),
    "dd": ("dt", "dd"),
    "tr": ("tr", "td", "th"),
    "td": ("td", "th"),
    "th": ("td", "th"),
    "option": ("option",),
    "optgroup": ("option", "optgroup"),
    "thead": ("tbody", "thead", "tfoot", "tr", "td", "th"),
    "tbody": ("tbody", "thead", "tfoot", "tr", "td", "th"),
    "tfoot": ("tbody", "thead", "tfoot", "tr", "td", "th"),
}

# Elements dont la portee bloque une fermeture implicite : on ne remonte
# jamais au-dela en cherchant une balise ouvrante a fermer.
SCOPE_BARRIERS = frozenset({"html", "body", "table", "td", "th", "caption"})

# Contenu autorise directement dans <head>.
HEAD_ONLY = frozenset({"base", "basefont", "bgsound", "link", "meta", "title", "style", "script", "noscript"})


class TreeBuilder:
    def __init__(self, url=None):
        self.document = Document(url)
        self.open_elements = []
        self.head = None
        self.body = None
        self._in_head = True

    # -- Helpers de pile ---------------------------------------------------

    @property
    def current(self):
        return self.open_elements[-1] if self.open_elements else self.document

    def _has_open(self, names):
        """Cherche un element ouvert parmi `names`, sans franchir une barriere."""
        for element in reversed(self.open_elements):
            if element.tag in names:
                return True
            if element.tag in SCOPE_BARRIERS:
                return False
        return False

    def _close_until(self, names):
        """Depile jusqu'a avoir ferme le premier element de `names` inclus."""
        while self.open_elements:
            element = self.open_elements.pop()
            if element.tag in names:
                return True
        return False

    def _insert(self, node):
        self.current.append_child(node)
        return node

    # -- Squelette du document --------------------------------------------

    def _ensure_html(self):
        if self.open_elements:
            return
        root = self.document.document_element
        if root is None:
            root = Element("html")
            self.document.append_child(root)
        self.open_elements.append(root)

    def _ensure_head(self):
        self._ensure_html()
        if self.head is None:
            self.head = Element("head")
            self.open_elements[0].append_child(self.head)
        return self.head

    def _ensure_body(self):
        self._ensure_html()
        if self.body is None:
            self._ensure_head()
            # On quitte le head des qu'un contenu de corps apparait.
            if self.head in self.open_elements:
                self._close_until({"head"})
            self.body = Element("body")
            self.open_elements[0].append_child(self.body)
            self.open_elements.append(self.body)
            self._in_head = False
        elif self.body not in self.open_elements and not self._body_is_ancestor():
            self.open_elements.append(self.body)
        return self.body

    def _body_is_ancestor(self):
        for element in self.open_elements:
            if element is self.body:
                return True
        return False

    # -- Traitement des jetons --------------------------------------------

    def process(self, token):
        kind = token.kind
        if kind == DOCTYPE:
            if self.document.doctype is None and not self.open_elements:
                self.document.doctype = token.data
            return
        if kind == COMMENT:
            target = self.current
            target.append_child(Comment(token.data))
            return
        if kind == TEXT:
            self._process_text(token)
            return
        if kind == START_TAG:
            self._process_start(token)
            return
        if kind == END_TAG:
            self._process_end(token)

    def _process_text(self, token):
        data = token.data
        if not self.open_elements:
            # Texte avant tout element : ignore s'il n'est qu'espaces.
            if not data.strip():
                return
            self._ensure_body()
        elif self._in_head and self.current.tag in ("html", "head"):
            if not data.strip():
                # Les espaces du head restent dans le head, sans creer de body.
                return
            self._ensure_body()
        self._insert(Text(data))

    def _process_start(self, token):
        name = token.name
        attrs = token.attrs

        if name == "html":
            self._ensure_html()
            # Les attributs d'un second <html> completent le premier.
            root = self.open_elements[0]
            for key, value in attrs.items():
                root.attrs.setdefault(key, value)
            return

        if name == "head":
            self._ensure_head()
            if self.head not in self.open_elements:
                self.open_elements.append(self.head)
            return

        if name == "body":
            body = self._ensure_body()
            for key, value in attrs.items():
                body.attrs.setdefault(key, value)
            return

        if name == "frameset":
            # Non supporte : traite comme un body pour ne pas perdre le contenu.
            self._ensure_body()
            return

        if self._in_head and name in HEAD_ONLY:
            self._ensure_head()
            if self.head not in self.open_elements:
                self.open_elements.append(self.head)
            element = Element(name, attrs)
            self.head.append_child(element)
            if name not in VOID_ELEMENTS and not token.self_closing:
                self.open_elements.append(element)
            return

        # Tout le reste appartient au corps.
        self._ensure_body()

        if name in CLOSES_PARAGRAPH and self._has_open({"p"}):
            self._close_until({"p"})

        implied = IMPLIED_END.get(name)
        if implied and self._has_open(implied):
            self._close_until(implied)

        # Un <table> imbrique directement dans un autre ferme le precedent.
        if name == "table" and self._has_open({"table"}):
            self._close_until({"table"})

        element = Element(name, attrs)
        self._insert(element)
        if name not in VOID_ELEMENTS and not token.self_closing:
            self.open_elements.append(element)

    def _process_end(self, token):
        name = token.name

        if name in ("html", "body"):
            # On ne depile pas : du contenu peut suivre une balise fermante hative.
            return

        if name == "head":
            if self.head in self.open_elements:
                self._close_until({"head"})
            self._in_head = False
            return

        if name in VOID_ELEMENTS:
            # `</br>` et consorts : ignore.
            return

        # La balise fermante doit correspondre a un element reellement ouvert,
        # sinon c'est une balise orpheline qu'on jette (comportement navigateur).
        for element in reversed(self.open_elements):
            if element.tag == name:
                self._close_until({name})
                return

    def finish(self):
        self._ensure_html()
        if self.head is None:
            self._ensure_head()
        if self.body is None:
            self._ensure_body()
        self.open_elements = []
        return self.document


def parse(source, url=None):
    """Analyse une chaine HTML et renvoie un `Document`."""
    if isinstance(source, (bytes, bytearray)):
        source = decode_bytes(source)
    builder = TreeBuilder(url)
    for token in tokenizer.tokenize(source):
        builder.process(token)
    return builder.finish()


def decode_bytes(raw, content_type=""):
    """Decode des octets HTML en texte, en devinant l'encodage.

    Ordre de priorite : BOM, `charset` de l'en-tete HTTP, `<meta charset>`
    dans les premiers kilo-octets, puis UTF-8 avec repli Windows-1252.
    """
    if raw.startswith(b"\xef\xbb\xbf"):
        return raw[3:].decode("utf-8", "replace")
    if raw.startswith(b"\xff\xfe"):
        return raw[2:].decode("utf-16-le", "replace")
    if raw.startswith(b"\xfe\xff"):
        return raw[2:].decode("utf-16-be", "replace")

    charset = ""
    lowered = content_type.lower()
    if "charset=" in lowered:
        charset = lowered.split("charset=", 1)[1].split(";")[0].strip().strip("\"'")

    if not charset:
        head = raw[:4096].lower()
        idx = head.find(b"charset")
        if idx >= 0:
            chunk = head[idx + 7 : idx + 48]
            chunk = chunk.lstrip(b"= \t\"'")
            end = 0
            while end < len(chunk) and (
                chunk[end : end + 1].isalnum() or chunk[end : end + 1] in (b"-", b"_")
            ):
                end += 1
            charset = chunk[:end].decode("ascii", "ignore")

    for candidate in (charset, "utf-8", "cp1252", "latin-1"):
        if not candidate:
            continue
        try:
            return raw.decode(candidate)
        except (UnicodeDecodeError, LookupError):
            continue
    return raw.decode("utf-8", "replace")
