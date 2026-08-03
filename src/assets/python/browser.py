"""Navigateur Web simpliste pour Bouchaud OS.

Reprend la structure du tutoriel « How to build a browser using Python »
(scientyficworld.org) : une classe fenetre qui tient des onglets, une barre
d'adresse, et les quatre actions de navigation — precedent, suivant,
recharger, accueil.

Le tutoriel affiche les pages avec `QWebEngineView`, c'est-a-dire Chromium
embarque par Qt. Rien de tout cela n'existe ici : Bouchaud OS est un noyau
`no_std` sans libc, sans C++, sans serveur graphique, et son `pip` n'installe
que des paquets purs Python. Le rendu passe donc par la console du noyau, et
le reseau par `/dev/web`, ou le noyau prete sa pile TCP/TLS.

Le but de ce programme est de verifier que **toutes les couches de l'OS
s'enchainent** :

    interpreteur Python -> pont WASI -> RAMFS -> pile TCP/TLS
        -> retour a Python -> extraction du HTML -> console

Usage :
    pybrowser                  # accueil, puis invite interactive
    pybrowser example.com      # ouvre une page
    pybrowser --check          # diagnostic couche par couche
"""

import sys

DEVICE = "/dev/web"
HOME = "http://example.com"

# Elements dont le contenu ne s'affiche pas.
SKIPPED = ("script", "style", "head", "title", "meta", "link")


# ---------------------------------------------------------------------------
# Reseau : le noyau fait le travail, on lui parle par un fichier
# ---------------------------------------------------------------------------


def fetch(url):
    """Recupere une URL. Renvoie (statut, url_finale, type, corps)."""
    try:
        with open(DEVICE, "w") as device:
            device.write(url + "\n")
        with open(DEVICE, "r") as device:
            raw = device.read()
    except OSError as exc:
        return 0, url, "", "peripherique %s indisponible : %s" % (DEVICE, exc)

    head, _, body = raw.partition("\n\n")
    status, final, ctype = 0, url, "?"
    for line in head.split("\n"):
        if line.startswith("STATUS "):
            try:
                status = int(line[7:].strip())
            except ValueError:
                status = 0
        elif line.startswith("URL "):
            final = line[4:].strip() or url
        elif line.startswith("TYPE "):
            ctype = line[5:].strip()
    return status, final, ctype, body


# ---------------------------------------------------------------------------
# Rendu : extraction du texte et des liens
# ---------------------------------------------------------------------------


def render(html, width=76):
    """Transforme du HTML en texte lisible. Renvoie (titre, lignes, liens)."""
    title = ""
    start = html.lower().find("<title>")
    if start >= 0:
        end = html.lower().find("</title>", start)
        if end > start:
            title = unescape(html[start + 7:end]).strip()

    links = []
    words = []
    i = 0
    length = len(html)
    skip_until = None

    while i < length:
        lt = html.find("<", i)
        if lt < 0:
            if skip_until is None:
                words.extend(unescape(html[i:]).split())
            break

        if skip_until is None:
            words.extend(unescape(html[i:lt]).split())

        gt = html.find(">", lt)
        if gt < 0:
            break
        tag = html[lt + 1:gt]
        name = tag.split()[0].lower() if tag.split() else ""
        i = gt + 1

        if name.startswith("/"):
            if skip_until == name[1:]:
                skip_until = None
            elif name[1:] in ("p", "div", "li", "tr", "h1", "h2", "h3", "br"):
                words.append("\n")
            continue

        if skip_until is not None:
            continue
        if name in SKIPPED:
            skip_until = name
            continue
        if name in ("p", "div", "li", "tr", "br", "h1", "h2", "h3", "hr"):
            words.append("\n")
        if name == "a":
            href = attribute(tag, "href")
            if href:
                links.append(href)
                words.append("[%d]" % len(links))

    # Mise en lignes a la largeur demandee.
    lines = []
    current = ""
    for word in words:
        if word == "\n":
            if current:
                lines.append(current)
                current = ""
            continue
        if len(current) + len(word) + 1 > width:
            lines.append(current)
            current = word
        else:
            current = (current + " " + word) if current else word
    if current:
        lines.append(current)
    return title, lines, links


def attribute(tag, name):
    """Valeur d'un attribut dans le texte brut d'une balise."""
    lowered = tag.lower()
    at = lowered.find(name + "=")
    if at < 0:
        return ""
    rest = tag[at + len(name) + 1:]
    if not rest:
        return ""
    if rest[0] in "\"'":
        quote = rest[0]
        end = rest.find(quote, 1)
        return rest[1:end] if end > 0 else rest[1:]
    return rest.split()[0] if rest.split() else ""


def unescape(text):
    """Les quelques entites qu'on rencontre partout."""
    if "&" not in text:
        return text
    for entity, char in (("&amp;", "&"), ("&lt;", "<"), ("&gt;", ">"),
                         ("&quot;", '"'), ("&#39;", "'"), ("&apos;", "'"),
                         ("&nbsp;", " ")):
        text = text.replace(entity, char)
    return text


def absolute(href, base):
    """Resolution d'un lien relatif contre l'URL courante."""
    if "://" in href or href.startswith(("about:", "mailto:", "javascript:")):
        return href
    scheme, _, rest = base.partition("://")
    host = rest.split("/")[0] if rest else ""
    if href.startswith("//"):
        return scheme + ":" + href
    if href.startswith("/"):
        return "%s://%s%s" % (scheme, host, href)
    directory = base[:base.rfind("/") + 1] if "/" in rest else base + "/"
    return directory + href


# ---------------------------------------------------------------------------
# Le navigateur : meme structure que la MainWindow du tutoriel
# ---------------------------------------------------------------------------


class Tab:
    """Un onglet : sa pile d'historique et la page affichee."""

    def __init__(self):
        self.history = []
        self.index = -1
        self.title = "Nouvel onglet"
        self.lines = []
        self.links = []

    @property
    def url(self):
        return self.history[self.index] if 0 <= self.index < len(self.history) else ""


class Browser:
    """Equivalent de `MainWindow` : onglets, barre d'adresse, navigation."""

    def __init__(self, home=HOME, width=76):
        self.tabs = []
        self.current = 0
        self.width = width
        self.home_url = home
        self.add_new_tab()

    # -- Onglets (add_new_tab / close_current_tab du tutoriel) -------------

    @property
    def tab(self):
        return self.tabs[self.current]

    def add_new_tab(self, url=None):
        self.tabs.append(Tab())
        self.current = len(self.tabs) - 1
        if url:
            self.navigate_to_url(url)
        return self.tab

    def close_current_tab(self):
        if len(self.tabs) <= 1:
            print("browser: dernier onglet, fermeture refusee")
            return
        del self.tabs[self.current]
        self.current = min(self.current, len(self.tabs) - 1)
        self.update_urlbar()

    def switch_tab(self, index):
        if 0 <= index < len(self.tabs):
            self.current = index
            self.update_urlbar()
        else:
            print("browser: aucun onglet %d" % index)

    # -- Navigation (navigate_to_url / navigate_home du tutoriel) ----------

    def navigate_to_url(self, text, record=True):
        url = self.qualify(text)
        print("chargement de %s ..." % url)
        status, final, ctype, body = fetch(url)

        if status == 0:
            print("echec : %s" % body.strip().split("\n")[0])
            return False
        if "html" not in ctype and "xml" not in ctype and not body.lstrip().startswith("<"):
            print("contenu non affichable (%s, %d octets)" % (ctype, len(body)))
            return False

        title, lines, links = render(body, self.width)
        tab = self.tab
        tab.title = title or final
        tab.lines = lines
        tab.links = links
        if record:
            del tab.history[tab.index + 1:]
            tab.history.append(final)
            tab.index = len(tab.history) - 1
        self.render_page()
        return True

    def navigate_home(self):
        return self.navigate_to_url(self.home_url)

    def navigate_back(self):
        tab = self.tab
        if tab.index <= 0:
            print("browser: pas de page precedente")
            return
        tab.index -= 1
        self.navigate_to_url(tab.url, record=False)

    def navigate_forward(self):
        tab = self.tab
        if tab.index >= len(tab.history) - 1:
            print("browser: pas de page suivante")
            return
        tab.index += 1
        self.navigate_to_url(tab.url, record=False)

    def reload(self):
        if self.tab.url:
            self.navigate_to_url(self.tab.url, record=False)
        else:
            self.navigate_home()

    def follow(self, number):
        tab = self.tab
        if not (1 <= number <= len(tab.links)):
            print("browser: aucun lien [%d]" % number)
            return
        self.navigate_to_url(absolute(tab.links[number - 1], tab.url))

    def qualify(self, text):
        """Barre d'adresse : complete le schema si l'utilisateur l'omet."""
        text = text.strip()
        if "://" not in text:
            return "http://" + text
        return text

    # -- Affichage (update_urlbar + la vue du tutoriel) --------------------

    def update_urlbar(self):
        tab = self.tab
        print("[onglet %d/%d] %s" % (self.current + 1, len(self.tabs), tab.url or "(vide)"))

    def render_page(self):
        tab = self.tab
        bar = "=" * self.width
        print(bar)
        print(tab.title)
        print(tab.url)
        print(bar)
        for line in tab.lines:
            print(line)
        if tab.links:
            print("-" * self.width)
            for number, href in enumerate(tab.links[:30], 1):
                print("[%2d] %s" % (number, href))

    # -- Boucle interactive (l'equivalent de app.exec_()) ------------------

    def run(self):
        print("")
        print("commandes : <url> | <n> suivre un lien | b precedent | f suivant")
        print("            r recharger | h accueil | t nouvel onglet | w fermer")
        print("            <Entree seule> pour quitter")
        while True:
            try:
                line = input("\nbrowser> ").strip()
            except (EOFError, KeyboardInterrupt):
                return 0
            if not line:
                return 0
            if line.isdigit():
                self.follow(int(line))
            elif line == "b":
                self.navigate_back()
            elif line == "f":
                self.navigate_forward()
            elif line == "r":
                self.reload()
            elif line == "h":
                self.navigate_home()
            elif line == "t":
                self.add_new_tab()
                self.update_urlbar()
            elif line == "w":
                self.close_current_tab()
            elif line.startswith("s "):
                self.switch_tab(int(line[2:]) - 1 if line[2:].isdigit() else -1)
            else:
                self.navigate_to_url(line)


# ---------------------------------------------------------------------------
# Diagnostic : chaque couche de l'OS, une par une
# ---------------------------------------------------------------------------


def check(url=HOME):
    """Verifie la chaine complete et dit ou elle casse."""
    results = []

    results.append(("interpreteur Python", True,
                    "%s %s" % (sys.implementation.name, sys.version.split()[0])))

    # Pont WASI -> RAMFS, en lecture.
    try:
        with open("/etc/passwd") as handle:
            handle.read(1)
        results.append(("pont WASI / RAMFS (lecture)", True, "/etc/passwd lisible"))
    except OSError as exc:
        results.append(("pont WASI / RAMFS (lecture)", False, str(exc)))

    # Pont WASI -> RAMFS, en ecriture.
    try:
        with open("/tmp/.webcheck", "w") as handle:
            handle.write("ok")
        with open("/tmp/.webcheck") as handle:
            written = handle.read()
        results.append(("pont WASI / RAMFS (ecriture)", written == "ok", "/tmp accessible"))
    except OSError as exc:
        results.append(("pont WASI / RAMFS (ecriture)", False, str(exc)))

    # Peripherique reseau.
    try:
        open(DEVICE).close()
        results.append(("peripherique %s" % DEVICE, True, "present"))
        device_ok = True
    except OSError as exc:
        results.append(("peripherique %s" % DEVICE, False, str(exc)))
        device_ok = False

    # Pile reseau du noyau.
    body = ""
    if device_ok:
        status, final, ctype, body = fetch(url)
        if status:
            results.append(("pile TCP/TLS du noyau", True,
                            "%s -> %d, %s, %d octets" % (final, status, ctype, len(body))))
        else:
            results.append(("pile TCP/TLS du noyau", False,
                            body.strip().split("\n")[0] or "aucune reponse"))
    else:
        results.append(("pile TCP/TLS du noyau", False, "peripherique absent"))

    # Extraction du HTML.
    if body:
        title, lines, links = render(body)
        ok = bool(lines)
        results.append(("extraction HTML", ok,
                        "titre=%r, %d lignes, %d liens" % (title, len(lines), len(links))))
    else:
        results.append(("extraction HTML", False, "pas de corps a analyser"))

    # Sortie console.
    results.append(("sortie console", True, "cette ligne s'affiche"))

    print("=" * 64)
    print("Diagnostic des couches - affichage de contenu Web")
    print("=" * 64)
    failures = 0
    for name, ok, detail in results:
        if not ok:
            failures += 1
        print("  [%s] %-30s %s" % ("OK" if ok else "KO", name, detail))
    print("=" * 64)
    if failures:
        print("%d couche(s) en echec." % failures)
    else:
        print("Toutes les couches repondent : l'OS sait afficher du contenu Web.")
    return 1 if failures else 0


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    width = 76
    target = None

    index = 0
    while index < len(argv):
        arg = argv[index]
        if arg in ("-h", "--help"):
            print(__doc__)
            return 0
        if arg == "--check":
            return check(argv[index + 1] if index + 1 < len(argv) else HOME)
        if arg == "--width" and index + 1 < len(argv):
            index += 1
            width = int(argv[index])
        elif not arg.startswith("-"):
            target = arg
        index += 1

    browser = Browser(width=width)
    if target:
        browser.navigate_to_url(target)
    else:
        browser.navigate_home()
    return browser.run()


if __name__ == "__main__":
    sys.exit(main())
