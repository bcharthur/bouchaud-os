"""Tokenizer HTML.

Balaye la source et emet des jetons : doctype, balise ouvrante, balise
fermante, texte, commentaire. C'est une machine a etats simplifiee par rapport
a la section 13.2.5 du standard HTML, mais elle couvre ce que le Web reel
produit : attributs non quotes, balises auto-fermantes, contenu brut de
`<script>`/`<style>`, commentaires et sections CDATA.
"""

from . import entities

# Types de jetons.
DOCTYPE = "doctype"
START_TAG = "start"
END_TAG = "end"
TEXT = "text"
COMMENT = "comment"

RAW_TEXT_ELEMENTS = frozenset({"script", "style", "textarea", "title", "xmp", "iframe", "noscript"})

_SPACE = " \t\n\r\f"


class Token:
    __slots__ = ("kind", "name", "attrs", "data", "self_closing")

    def __init__(self, kind, name="", attrs=None, data="", self_closing=False):
        self.kind = kind
        self.name = name
        self.attrs = attrs if attrs is not None else {}
        self.data = data
        self.self_closing = self_closing

    def __repr__(self):
        if self.kind == TEXT:
            preview = self.data if len(self.data) <= 24 else self.data[:21] + "..."
            return "Token(text %r)" % preview
        if self.kind == START_TAG:
            return "Token(<%s> %r)" % (self.name, self.attrs)
        if self.kind == END_TAG:
            return "Token(</%s>)" % self.name
        return "Token(%s %r)" % (self.kind, self.data[:24])


def tokenize(source):
    """Generateur de `Token` sur la chaine `source`."""
    i = 0
    length = len(source)
    text_start = 0

    def flush_text(upto):
        if upto > text_start:
            raw = source[text_start:upto]
            if raw:
                return Token(TEXT, data=entities.decode(raw))
        return None

    while i < length:
        ch = source[i]
        if ch != "<":
            i += 1
            continue

        # `<` isole (suivi d'un espace ou en fin) : ce n'est pas une balise.
        nxt = source[i + 1] if i + 1 < length else ""
        if not (nxt.isalpha() or nxt in "/!?"):
            i += 1
            continue

        pending = flush_text(i)
        if pending is not None:
            yield pending

        # Commentaire, doctype ou CDATA.
        if source.startswith("<!--", i):
            end = source.find("-->", i + 4)
            if end < 0:
                yield Token(COMMENT, data=source[i + 4 :])
                i = length
                text_start = i
                break
            yield Token(COMMENT, data=source[i + 4 : end])
            i = end + 3
            text_start = i
            continue

        if source.startswith("<![CDATA[", i):
            end = source.find("]]>", i + 9)
            if end < 0:
                yield Token(TEXT, data=source[i + 9 :])
                i = length
                text_start = i
                break
            yield Token(TEXT, data=source[i + 9 : end])
            i = end + 3
            text_start = i
            continue

        if source.startswith("<!", i) or source.startswith("<?", i):
            end = source.find(">", i)
            if end < 0:
                i = length
                text_start = i
                break
            body = source[i + 2 : end]
            if body[:7].lower() == "doctype":
                yield Token(DOCTYPE, data=body[7:].strip())
            else:
                yield Token(COMMENT, data=body)
            i = end + 1
            text_start = i
            continue

        # Balise fermante.
        if source.startswith("</", i):
            j = i + 2
            name_start = j
            while j < length and (source[j].isalnum() or source[j] in "-_:"):
                j += 1
            name = source[name_start:j].lower()
            end = source.find(">", j)
            if end < 0:
                i = length
                text_start = i
                break
            if name:
                yield Token(END_TAG, name=name)
            i = end + 1
            text_start = i
            continue

        # Balise ouvrante.
        token, i = _read_start_tag(source, i)
        if token is None:
            # `<` qui ne commence pas une balise valide : texte litteral.
            i += 1
            continue
        yield token
        text_start = i

        # Contenu brut : on cherche directement la balise fermante.
        if token.name in RAW_TEXT_ELEMENTS and not token.self_closing:
            close = _find_closing(source, i, token.name)
            raw = source[i:close]
            if raw:
                # Le contenu de textarea et title est decode, pas celui de script/style.
                if token.name in ("textarea", "title"):
                    yield Token(TEXT, data=entities.decode(raw))
                else:
                    yield Token(TEXT, data=raw)
            end_tag = source.find(">", close)
            i = (end_tag + 1) if end_tag >= 0 else length
            yield Token(END_TAG, name=token.name)
            text_start = i

    tail = flush_text(length)
    if tail is not None:
        yield tail


def _find_closing(source, start, name):
    """Position du `</name` correspondant, ou la fin de la source."""
    needle = "</" + name
    i = start
    lowered = source.lower()
    while True:
        idx = lowered.find(needle, i)
        if idx < 0:
            return len(source)
        after = idx + len(needle)
        # Doit etre suivi de `>`, d'un espace ou d'un `/` — pas d'une lettre.
        if after >= len(source) or not (source[after].isalnum() or source[after] in "-_"):
            return idx
        i = after


def _read_start_tag(source, i):
    """Lit une balise ouvrante a partir de `<`. Renvoie (Token|None, position)."""
    length = len(source)
    j = i + 1
    name_start = j
    while j < length and (source[j].isalnum() or source[j] in "-_:"):
        j += 1
    name = source[name_start:j].lower()
    if not name:
        return None, i

    attrs = {}
    self_closing = False
    while j < length:
        # Saute les espaces entre attributs.
        while j < length and source[j] in _SPACE:
            j += 1
        if j >= length:
            break
        ch = source[j]
        if ch == ">":
            j += 1
            break
        if ch == "/":
            if j + 1 < length and source[j + 1] == ">":
                self_closing = True
                j += 2
                break
            j += 1
            continue

        # Nom d'attribut.
        attr_start = j
        while j < length and source[j] not in _SPACE and source[j] not in "=/>":
            j += 1
        attr_name = source[attr_start:j].lower()
        if not attr_name:
            j += 1
            continue

        while j < length and source[j] in _SPACE:
            j += 1

        value = ""
        if j < length and source[j] == "=":
            j += 1
            while j < length and source[j] in _SPACE:
                j += 1
            if j < length and source[j] in "\"'":
                quote = source[j]
                j += 1
                value_start = j
                end = source.find(quote, j)
                if end < 0:
                    value = source[value_start:]
                    j = length
                else:
                    value = source[value_start:end]
                    j = end + 1
            else:
                value_start = j
                while j < length and source[j] not in _SPACE and source[j] != ">":
                    j += 1
                value = source[value_start:j]
            value = entities.decode(value)
        else:
            # Attribut booleen (`disabled`) : valeur vide, presence significative.
            value = ""

        # Premiere occurrence gagnante, comme le standard.
        if attr_name not in attrs:
            attrs[attr_name] = value

    return Token(START_TAG, name=name, attrs=attrs, self_closing=self_closing), j
