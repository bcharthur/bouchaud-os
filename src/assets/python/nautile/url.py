"""Analyse et resolution d'URL.

Implementation autonome (pas d'`urllib`) : le navigateur doit tourner sur des
interpreteurs minimaux comme RustPython embarque dans Bouchaud OS, ou la
stdlib figee peut etre partielle. Suit WHATWG URL pour les cas courants du
navigateur : schemes speciaux, ports par defaut, resolution relative,
normalisation des segments `.` et `..`, encodage pourcent.
"""

DEFAULT_PORTS = {"http": 80, "https": 443, "ws": 80, "wss": 443, "ftp": 21}

SPECIAL_SCHEMES = frozenset(DEFAULT_PORTS)

# Schemes qui n'ont pas d'autorite (`//host`) : tout ce qui suit `:` est opaque.
OPAQUE_SCHEMES = frozenset({"about", "data", "javascript", "mailto", "blob"})

_UNRESERVED = frozenset(
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ" "abcdefghijklmnopqrstuvwxyz" "0123456789" "-._~"
)


class URL:
    """URL decomposee. Immuable en pratique : on reconstruit plutot que muter."""

    __slots__ = ("scheme", "host", "port", "path", "query", "fragment")

    def __init__(self, scheme="", host="", port=None, path="", query="", fragment=""):
        self.scheme = scheme
        self.host = host
        self.port = port
        self.path = path
        self.query = query
        self.fragment = fragment

    @property
    def is_special(self):
        return self.scheme in SPECIAL_SCHEMES

    @property
    def is_opaque(self):
        return self.scheme in OPAQUE_SCHEMES

    @property
    def effective_port(self):
        if self.port is not None:
            return self.port
        return DEFAULT_PORTS.get(self.scheme)

    @property
    def origin(self):
        """Origine au sens Same-Origin Policy : (scheme, host, port)."""
        if self.is_opaque or not self.host:
            return None
        return (self.scheme, self.host, self.effective_port)

    def request_target(self):
        """Cible de requete HTTP : chemin + query, jamais vide."""
        path = self.path or "/"
        if self.query:
            return path + "?" + self.query
        return path

    def __str__(self):
        if self.is_opaque:
            out = self.scheme + ":" + self.path
            if self.query:
                out += "?" + self.query
            if self.fragment:
                out += "#" + self.fragment
            return out
        out = self.scheme + "://" if self.scheme else "//"
        out += self.host
        # On n'affiche le port que s'il differe du port par defaut du scheme.
        if self.port is not None and self.port != DEFAULT_PORTS.get(self.scheme):
            out += ":" + str(self.port)
        out += self.path or "/"
        if self.query:
            out += "?" + self.query
        if self.fragment:
            out += "#" + self.fragment
        return out

    def __repr__(self):
        return "URL(%r)" % str(self)

    def __eq__(self, other):
        return isinstance(other, URL) and str(self) == str(other)

    def __hash__(self):
        return hash(str(self))


def _split_scheme(text):
    """Renvoie (scheme, reste) ou (None, texte) si aucun scheme valide."""
    for i, ch in enumerate(text):
        if ch == ":":
            if i == 0:
                return None, text
            scheme = text[:i].lower()
            # Un scheme commence par une lettre puis [a-z0-9+-.]
            if not scheme[0].isalpha():
                return None, text
            for c in scheme[1:]:
                if not (c.isalnum() or c in "+-."):
                    return None, text
            return scheme, text[i + 1 :]
        if not (ch.isalnum() or ch in "+-."):
            return None, text
    return None, text


def normalize_path(path):
    """Supprime les segments `.` et `..` (RFC 3986 section 5.2.4)."""
    if not path:
        return ""
    absolute = path.startswith("/")
    trailing = path.endswith("/") or path.endswith("/.") or path.endswith("/..")
    out = []
    for seg in path.split("/"):
        if seg == "" or seg == ".":
            continue
        if seg == "..":
            # On ne remonte jamais au-dessus de la racine.
            if out:
                out.pop()
            continue
        out.append(seg)
    result = "/".join(out)
    if absolute:
        result = "/" + result
    if trailing and not result.endswith("/"):
        result += "/"
    return result


def parse(text, base=None):
    """Analyse `text`, en le resolvant contre `base` s'il est relatif.

    `base` accepte une `URL` ou une chaine. Renvoie toujours une `URL` ;
    leve `ValueError` si l'entree est vide ou inanalysable.
    """
    if text is None:
        raise ValueError("URL vide")
    text = text.strip().replace("\t", "").replace("\n", "").replace("\r", "")
    if not text:
        raise ValueError("URL vide")
    if isinstance(base, str):
        base = parse(base)

    scheme, rest = _split_scheme(text)

    # Reference relative : on herite du scheme et de l'autorite de la base.
    if scheme is None:
        if base is None:
            raise ValueError("URL relative sans base : %r" % text)
        return _resolve_relative(text, base)

    if scheme in OPAQUE_SCHEMES:
        body, fragment = _cut(rest, "#")
        body, query = _cut(body, "?")
        return URL(scheme=scheme, path=body, query=query, fragment=fragment)

    # Scheme suivi de `//` : autorite presente.
    if rest.startswith("//"):
        rest = rest[2:]
    elif scheme in SPECIAL_SCHEMES:
        # `http:example.com/x` est tolere par les navigateurs.
        rest = rest.lstrip("/")
    else:
        # Scheme inconnu sans autorite : traite comme opaque.
        body, fragment = _cut(rest, "#")
        body, query = _cut(body, "?")
        return URL(scheme=scheme, path=body, query=query, fragment=fragment)

    authority, tail = _cut_first(rest, "/?#")
    host, port = _split_host_port(authority)
    path, query, fragment = _split_path(tail)
    return URL(scheme, host, port, normalize_path(path) or "/", query, fragment)


def _resolve_relative(text, base):
    if text.startswith("//"):
        # Reference protocol-relative : `//cdn.example/x` herite du scheme.
        authority, tail = _cut_first(text[2:], "/?#")
        host, port = _split_host_port(authority)
        path, query, fragment = _split_path(tail)
        return URL(base.scheme, host, port, normalize_path(path) or "/", query, fragment)

    if text.startswith("#"):
        return URL(base.scheme, base.host, base.port, base.path, base.query, text[1:])

    if text.startswith("?"):
        query, fragment = _cut(text[1:], "#")
        return URL(base.scheme, base.host, base.port, base.path, query, fragment)

    path, query, fragment = _split_path(text)
    if path.startswith("/"):
        merged = path
    else:
        # On repart du repertoire de la base, pas du document lui-meme.
        base_dir = base.path[: base.path.rfind("/") + 1] or "/"
        merged = base_dir + path
    return URL(
        base.scheme, base.host, base.port, normalize_path(merged) or "/", query, fragment
    )


def _cut(text, sep):
    """Coupe au premier `sep` : renvoie (avant, apres) — apres vide si absent."""
    idx = text.find(sep)
    if idx < 0:
        return text, ""
    return text[:idx], text[idx + 1 :]


def _cut_first(text, seps):
    """Coupe au premier caractere de `seps`, en le gardant dans le reste."""
    for i, ch in enumerate(text):
        if ch in seps:
            return text[:i], text[i:]
    return text, ""


def _split_host_port(authority):
    # On ignore les identifiants `user:pass@` (jamais envoyes par Nautile).
    at = authority.rfind("@")
    if at >= 0:
        authority = authority[at + 1 :]
    if authority.startswith("["):
        # IPv6 litteral : `[::1]:8080`
        end = authority.find("]")
        if end >= 0:
            host = authority[: end + 1]
            tail = authority[end + 1 :]
            if tail.startswith(":") and tail[1:].isdigit():
                return host, int(tail[1:])
            return host, None
    if ":" in authority:
        host, _, port = authority.rpartition(":")
        if port.isdigit():
            return host.lower(), int(port)
    return authority.lower(), None


def _split_path(text):
    path, fragment = _cut(text, "#")
    path, query = _cut(path, "?")
    return path, query, fragment


def percent_encode(text, safe=""):
    """Encode en pourcent tout sauf les caracteres non reserves et `safe`."""
    keep = _UNRESERVED.union(safe)
    out = []
    for byte in text.encode("utf-8"):
        ch = chr(byte)
        if ch in keep:
            out.append(ch)
        else:
            out.append("%%%02X" % byte)
    return "".join(out)


def percent_decode(text):
    """Decode les sequences `%XX`. Les sequences invalides sont conservees."""
    if "%" not in text:
        return text
    raw = bytearray()
    i = 0
    data = text.encode("utf-8")
    while i < len(data):
        if data[i] == 0x25 and i + 2 < len(data):
            try:
                raw.append(int(data[i + 1 : i + 3].decode("ascii"), 16))
                i += 3
                continue
            except ValueError:
                pass
        raw.append(data[i])
        i += 1
    return raw.decode("utf-8", "replace")


def encode_form(pairs):
    """Encode des paires en `application/x-www-form-urlencoded`."""
    out = []
    for name, value in pairs:
        # L'espace devient `+` dans ce format, contrairement au reste de l'URL.
        key = percent_encode(str(name), safe="*").replace("%20", "+")
        val = percent_encode(str(value), safe="*").replace("%20", "+")
        out.append(key + "=" + val)
    return "&".join(out)


def decode_form(text):
    """Decode une chaine `a=1&b=2` en liste de paires."""
    pairs = []
    for chunk in text.split("&"):
        if not chunk:
            continue
        name, _, value = chunk.partition("=")
        pairs.append(
            (percent_decode(name.replace("+", " ")), percent_decode(value.replace("+", " ")))
        )
    return pairs


def same_origin(a, b):
    """Vrai si les deux URL partagent la meme origine (SOP)."""
    origin_a = a.origin if isinstance(a, URL) else parse(a).origin
    origin_b = b.origin if isinstance(b, URL) else parse(b).origin
    return origin_a is not None and origin_a == origin_b
