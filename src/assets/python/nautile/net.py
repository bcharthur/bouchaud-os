"""Reseau : recuperation des ressources.

Le transport est enfichable. Par defaut on parle HTTP directement sur une
socket — cela evite `urllib`, absent ou partiel sur les interpreteurs
minimaux. Bouchaud OS enregistre a la place un transport qui delegue a la
pile TCP/TLS du noyau (voir `nautile.backends.bouchaud`).
"""

import time

from . import url as urlmod

USER_AGENT = "Mozilla/5.0 (compatible; Nautile/2.0; +https://github.com/bcharthur/nautile-navigateur)"

# Limites : une page hostile ne doit ni saturer la memoire ni bloquer l'UI.
MAX_BODY = 8 * 1024 * 1024
MAX_REDIRECTS = 8
DEFAULT_TIMEOUT = 15.0


class Response:
    __slots__ = ("url", "status", "headers", "body", "error", "elapsed", "trace")

    def __init__(self, url="", status=0, headers=None, body=b"", error=None,
                 elapsed=0.0, trace=None):
        self.url = url
        self.status = status
        self.headers = headers if headers is not None else {}
        self.body = body
        self.error = error
        self.elapsed = elapsed
        self.trace = trace if trace is not None else []

    @property
    def ok(self):
        return self.error is None and 200 <= self.status < 400

    @property
    def content_type(self):
        return self.headers.get("content-type", "")

    @property
    def is_html(self):
        ctype = self.content_type.lower()
        if "html" in ctype or "xml" in ctype:
            return True
        if not ctype and self.body[:512].lstrip()[:1] == b"<":
            return True
        return False

    def text(self):
        from .html.parser import decode_bytes

        return decode_bytes(self.body, self.content_type)

    def __repr__(self):
        return "Response(%s, %d, %d octets)" % (self.url, self.status, len(self.body))


class Transport:
    """Interface d'un transport. `fetch` renvoie une `Response`."""

    def fetch(self, parsed, method="GET", headers=None, body=None, timeout=DEFAULT_TIMEOUT):
        raise NotImplementedError


class SocketTransport(Transport):
    """HTTP/1.1 sur socket, TLS via `ssl` quand il est disponible."""

    def fetch(self, parsed, method="GET", headers=None, body=None, timeout=DEFAULT_TIMEOUT):
        import socket

        host = parsed.host
        port = parsed.effective_port or (443 if parsed.scheme == "https" else 80)
        trace = []

        request_headers = {
            "Host": host if port in (80, 443) else "%s:%d" % (host, port),
            "User-Agent": USER_AGENT,
            "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            "Accept-Language": "fr,en;q=0.8",
            "Accept-Encoding": "gzip, deflate",
            "Connection": "close",
        }
        if headers:
            request_headers.update(headers)
        if body:
            request_headers["Content-Length"] = str(len(body))

        lines = ["%s %s HTTP/1.1" % (method, parsed.request_target())]
        for name, value in request_headers.items():
            lines.append("%s: %s" % (name, value))
        request = ("\r\n".join(lines) + "\r\n\r\n").encode("utf-8")
        if body:
            request += body

        started = time.time()
        sock = None
        try:
            sock = socket.create_connection((host, port), timeout=timeout)
            trace.append("TCP %s:%d" % (host, port))
            if parsed.scheme == "https":
                import ssl

                context = ssl.create_default_context()
                sock = context.wrap_socket(sock, server_hostname=host)
                trace.append("TLS %s" % (sock.version() or "?"))
            sock.sendall(request)

            chunks = []
            total = 0
            while True:
                data = sock.recv(65536)
                if not data:
                    break
                chunks.append(data)
                total += len(data)
                if total > MAX_BODY * 2:
                    trace.append("reponse tronquee")
                    break
            raw = b"".join(chunks)
        except Exception as exc:
            return Response(
                url=str(parsed), error="%s: %s" % (type(exc).__name__, exc),
                elapsed=time.time() - started, trace=trace,
            )
        finally:
            if sock is not None:
                try:
                    sock.close()
                except Exception:
                    pass

        status, response_headers, payload = parse_http_response(raw)
        payload = decode_body(payload, response_headers)
        return Response(
            url=str(parsed), status=status, headers=response_headers, body=payload,
            elapsed=time.time() - started, trace=trace,
        )


def parse_http_response(raw):
    """Decoupe une reponse brute en (statut, en-tetes, corps)."""
    separator = raw.find(b"\r\n\r\n")
    if separator < 0:
        separator = raw.find(b"\n\n")
        head = raw[:separator] if separator >= 0 else raw
        body = raw[separator + 2 :] if separator >= 0 else b""
    else:
        head = raw[:separator]
        body = raw[separator + 4 :]

    lines = head.replace(b"\r\n", b"\n").split(b"\n")
    status = 0
    if lines:
        parts = lines[0].split(b" ", 2)
        if len(parts) >= 2 and parts[1].isdigit():
            status = int(parts[1])

    headers = {}
    for line in lines[1:]:
        if not line:
            continue
        name, _, value = line.partition(b":")
        key = name.decode("latin-1").strip().lower()
        if not key:
            continue
        decoded = value.decode("latin-1").strip()
        if key == "set-cookie":
            # Seul en-tete qui peut legitimement apparaitre plusieurs fois :
            # on accumule au lieu d'ecraser, sinon on perd des cookies.
            headers.setdefault(key, []).append(decoded)
        else:
            headers[key] = decoded

    if headers.get("transfer-encoding", "").lower() == "chunked":
        body = dechunk(body)
    return status, headers, body


def dechunk(body):
    """Reassemble un corps `Transfer-Encoding: chunked`."""
    out = bytearray()
    position = 0
    length = len(body)
    while position < length:
        end = body.find(b"\r\n", position)
        if end < 0:
            break
        size_line = body[position:end].split(b";")[0].strip()
        try:
            size = int(size_line, 16)
        except ValueError:
            break
        if size == 0:
            break
        start = end + 2
        out += body[start : start + size]
        position = start + size + 2
        if len(out) > MAX_BODY:
            break
    return bytes(out)


def decode_body(body, headers):
    """Decompresse selon `Content-Encoding`, sans echouer si c'est impossible."""
    encoding = headers.get("content-encoding", "").lower().strip()
    if not encoding or not body:
        return body
    try:
        import zlib

        if "gzip" in encoding:
            return zlib.decompress(body, 16 + zlib.MAX_WBITS)
        if "deflate" in encoding:
            try:
                return zlib.decompress(body)
            except zlib.error:
                return zlib.decompress(body, -zlib.MAX_WBITS)
        if "br" in encoding:
            try:
                import brotli

                return brotli.decompress(body)
            except ImportError:
                # Sans decodeur brotli on renvoie le corps tel quel : le
                # parseur HTML produira une page vide plutot qu'une exception.
                return body
    except Exception:
        return body
    return body


class Session:
    """Etat reseau partage : transport, cookies, cache memoire."""

    def __init__(self, transport=None, cache_size=48):
        self.transport = transport if transport is not None else SocketTransport()
        self.cookies = {}
        self._cache = {}
        self._cache_order = []
        self.cache_size = cache_size

    # -- Cookies -----------------------------------------------------------

    def store_cookies(self, parsed, headers):
        raw = headers.get("set-cookie")
        if not raw:
            return
        values = raw if isinstance(raw, list) else [raw]
        jar = self.cookies.setdefault(parsed.host, {})
        for entry in values:
            pair = entry.split(";", 1)[0]
            name, _, value = pair.partition("=")
            name = name.strip()
            if name:
                jar[name] = value.strip()

    def cookie_header(self, parsed):
        jar = self.cookies.get(parsed.host)
        if not jar:
            return None
        return "; ".join("%s=%s" % item for item in jar.items())

    # -- Cache -------------------------------------------------------------

    def cached(self, key):
        return self._cache.get(key)

    def store(self, key, response):
        if key in self._cache:
            return
        self._cache[key] = response
        self._cache_order.append(key)
        while len(self._cache_order) > self.cache_size:
            oldest = self._cache_order.pop(0)
            self._cache.pop(oldest, None)

    # -- Requetes ----------------------------------------------------------

    def fetch(self, target, method="GET", headers=None, body=None,
              timeout=DEFAULT_TIMEOUT, use_cache=True):
        """Recupere une URL en suivant les redirections."""
        parsed = target if isinstance(target, urlmod.URL) else urlmod.parse(target)

        if use_cache and method == "GET":
            hit = self.cached(str(parsed))
            if hit is not None:
                return hit

        trace = []
        current = parsed
        for hop in range(MAX_REDIRECTS):
            request_headers = dict(headers) if headers else {}
            cookie = self.cookie_header(current)
            if cookie:
                request_headers["Cookie"] = cookie

            response = self.transport.fetch(
                current, method=method, headers=request_headers, body=body,
                timeout=timeout,
            )
            trace.extend(response.trace)
            self.store_cookies(current, response.headers)

            if response.error is not None:
                response.trace = trace
                return response

            if response.status in (301, 302, 303, 307, 308):
                location = response.headers.get("location")
                if not location:
                    break
                try:
                    current = urlmod.parse(location, base=current)
                except ValueError:
                    break
                trace.append("redirection %d -> %s" % (response.status, current))
                # Une redirection apres POST se poursuit en GET (RFC 9110).
                if response.status == 303 or (response.status == 302 and method == "POST"):
                    method = "GET"
                    body = None
                continue

            response.url = str(current)
            response.trace = trace
            if use_cache and method == "GET" and response.ok:
                self.store(str(parsed), response)
                if str(current) != str(parsed):
                    self.store(str(current), response)
            return response

        return Response(
            url=str(current), error="trop de redirections", trace=trace
        )


_default_session = None


def default_session():
    global _default_session
    if _default_session is None:
        _default_session = Session()
    return _default_session


def set_transport(transport):
    """Remplace le transport de la session par defaut (utilise par Bouchaud OS)."""
    default_session().transport = transport


def fetch(target, **kwargs):
    return default_session().fetch(target, **kwargs)
