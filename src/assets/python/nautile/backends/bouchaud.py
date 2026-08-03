"""Backend Bouchaud OS.

Dans Bouchaud OS, Python s'execute par RustPython compile en WebAssembly, et
la seule frontiere ouverte entre le module WASM et le noyau est le pont WASI
sur fichiers. Ce backend s'appuie donc sur trois pseudo-fichiers exposes par
le noyau dans le RAMFS :

    /dev/nautile/net      requetes reseau (la pile TCP/TLS du noyau)
    /dev/nautile/gpu      commandes de dessin (le framebuffer du noyau)
    /dev/nautile/events   evenements clavier et souris

Le protocole est du texte ligne a ligne : c'est ce qui se code le plus
simplement cote noyau `no_std`, et cela reste inspectable a la main.

Aucun de ces fichiers n'est requis : s'ils manquent, le backend le signale
proprement au lieu de planter, et le navigateur reste utilisable en mode
texte.
"""

import os

from .. import net as netmod
from ..browser import Browser
from ..display_list import Image, Rect, Text
from .base import Canvas, render

DEVICE_DIR = "/dev/nautile"
NET_DEVICE = DEVICE_DIR + "/net"
GPU_DEVICE = DEVICE_DIR + "/gpu"
EVENT_DEVICE = DEVICE_DIR + "/events"


def available():
    """Vrai si le noyau expose les peripheriques Nautile."""
    return os.path.exists(NET_DEVICE)


class KernelTransport(netmod.Transport):
    """Transport delegant a la pile reseau du noyau.

    On ecrit `GET <url>` dans `/dev/nautile/net`, puis on relit le meme
    fichier : le noyau y a depose une reponse HTTP brute, deja dechunkee et
    decompressee par ses propres decodeurs.
    """

    def fetch(self, parsed, method="GET", headers=None, body=None,
              timeout=netmod.DEFAULT_TIMEOUT):
        import time

        started = time.time()
        target = str(parsed)
        try:
            with open(NET_DEVICE, "w", encoding="utf-8") as device:
                device.write("%s %s\n" % (method, target))
            with open(NET_DEVICE, "rb") as device:
                raw = device.read()
        except OSError as exc:
            return netmod.Response(
                url=target, error="peripherique reseau indisponible : %s" % exc,
                elapsed=time.time() - started,
            )

        if not raw:
            return netmod.Response(
                url=target, error="aucune reponse du noyau",
                elapsed=time.time() - started,
            )

        status, response_headers, payload = netmod.parse_http_response(raw)
        payload = netmod.decode_body(payload, response_headers)
        return netmod.Response(
            url=target, status=status, headers=response_headers, body=payload,
            elapsed=time.time() - started, trace=["noyau %s" % parsed.scheme],
        )


class FramebufferCanvas(Canvas):
    """Canvas ecrivant des commandes de dessin vers le noyau.

    Les commandes sont accumulees puis envoyees en un seul bloc : un aller-
    retour par primitive couterait bien plus cher que le dessin lui-meme.
    """

    def __init__(self, stream):
        self.stream = stream
        self.commands = []

    def fill_rect(self, x, y, w, h, color, radius=0):
        if w <= 0 or h <= 0:
            return
        if radius > 0:
            self.commands.append(
                "round %d %d %d %d %d %d" % (int(x), int(y), int(w), int(h), radius, color)
            )
        else:
            self.commands.append(
                "rect %d %d %d %d %d" % (int(x), int(y), int(w), int(h), color)
            )

    def draw_text(self, x, y, text, color, size, bold=False, italic=False,
                  monospace=False, underline=False, line_through=False):
        if not text.strip():
            return
        flags = 0
        if bold:
            flags |= 1
        if italic:
            flags |= 2
        if monospace:
            flags |= 4
        if underline:
            flags |= 8
        if line_through:
            flags |= 16
        # Le texte finit la ligne : il peut contenir des espaces, jamais de
        # saut de ligne (le layout a deja decoupe en fragments).
        cleaned = text.replace("\n", " ").replace("\r", " ")
        self.commands.append(
            "text %d %d %d %d %d %s"
            % (int(x), int(y), int(size), flags, color, cleaned)
        )

    def draw_image(self, x, y, w, h, image):
        # Le noyau sait decoder PNG/JPEG/GIF/WebP : on lui passe la source et
        # il peint lui-meme, ou dessine un cadre s'il ne peut pas.
        self.commands.append(
            "image %d %d %d %d %s" % (int(x), int(y), int(w), int(h), image.src or "")
        )

    def clip_push(self, x, y, w, h):
        self.commands.append("clip %d %d %d %d" % (int(x), int(y), int(w), int(h)))

    def clip_pop(self):
        self.commands.append("unclip")

    def flush(self):
        self.commands.append("flush")
        self.stream.write("\n".join(self.commands) + "\n")
        self.stream.flush()
        self.commands = []


def read_event():
    """Lit un evenement du noyau. Renvoie (type, args) ou None."""
    try:
        with open(EVENT_DEVICE, "r", encoding="utf-8") as device:
            line = device.readline()
    except OSError:
        return None
    line = line.strip()
    if not line:
        return None
    parts = line.split()
    kind = parts[0]
    args = []
    for token in parts[1:]:
        try:
            args.append(int(token))
        except ValueError:
            args.append(token)
    return (kind, args)


def paint(browser, width, height):
    """Peint l'onglet actif sur le framebuffer du noyau."""
    page = browser.tab.page
    if page is None or page.display_list is None:
        return
    try:
        with open(GPU_DEVICE, "w", encoding="utf-8") as device:
            canvas = FramebufferCanvas(device)
            render(
                canvas, page.display_list, scroll=browser.tab.scroll,
                viewport=(width, height),
            )
            canvas.flush()
    except OSError as exc:
        print("nautile: framebuffer indisponible : %s" % exc)


def run(start_url=None, width=1024, height=700):
    """Boucle principale dans Bouchaud OS."""
    if not available():
        print("nautile: peripheriques noyau absents (%s)." % DEVICE_DIR)
        print("nautile: repli sur le rendu texte.")
        return run_text(start_url)

    netmod.set_transport(KernelTransport())
    browser = Browser(width=width, height=height)
    browser.navigate(start_url or "about:home")
    paint(browser, width, height)

    while True:
        event = read_event()
        if event is None:
            continue
        kind, args = event

        if kind == "quit":
            return 0
        if kind == "click" and len(args) >= 2:
            browser.click(args[0], args[1] + browser.tab.scroll)
        elif kind == "scroll" and args:
            browser.scroll_by(args[0])
        elif kind == "resize" and len(args) >= 2:
            width, height = args[0], args[1]
            browser.resize(width, height)
        elif kind == "navigate" and args:
            browser.navigate(str(args[0]))
        elif kind == "back":
            browser.go_back()
        elif kind == "forward":
            browser.go_forward()
        elif kind == "reload":
            browser.reload()
        elif kind == "home":
            browser.home()
        elif kind == "key" and args:
            _handle_key(browser, args[0])

        paint(browser, width, height)


def _handle_key(browser, code):
    tab = browser.tab
    if tab.focused_field is not None:
        if code == 8:
            tab.focused_field.value = tab.focused_field.value[:-1]
        elif code == 13:
            browser.submit()
        elif 32 <= code < 127:
            tab.focused_field.value += chr(code)
        return
    if code == 13:
        browser.scroll_by(60)


def run_text(start_url=None, columns=100):
    """Repli sans framebuffer : rendu texte sur la console du noyau."""
    from .text import dump

    if available():
        netmod.set_transport(KernelTransport())
    browser = Browser(width=columns * 8, height=600)
    page = browser.navigate(start_url or "about:home")
    print(dump(page, columns=columns))
    return 0
