"""Etat du navigateur : onglets, historique, navigation.

C'est la couche que pilote une interface graphique. Elle ne dessine rien :
elle produit des `DisplayList` et expose les actions d'un navigateur —
ouvrir, reculer, avancer, recharger, revenir a l'accueil. Un backend Qt, Tk
ou framebuffer se branche dessus sans rien savoir du moteur de rendu.
"""

from . import display_list as dl
from . import net, pages, style as stylemod, url as urlmod
from .html import parser as htmlparser
from .layout import LayoutEngine

# Moteurs de recherche disponibles depuis la barre d'adresse.
SEARCH_ENGINES = {
    "duckduckgo": "https://duckduckgo.com/html/?q=%s",
    "google": "https://www.google.com/search?q=%s",
    "bing": "https://www.bing.com/search?q=%s",
    "wikipedia": "https://fr.wikipedia.org/w/index.php?search=%s",
}

DEFAULT_SETTINGS = {
    "homepage": "about:home",
    "search-engine": "duckduckgo",
    "viewport-width": 1024,
    "viewport-height": 768,
    "load-stylesheets": True,
    "load-images": False,
    "javascript": False,
}

# Suffixes reconnus sans point d'interrogation : `exemple.fr` est une URL,
# `pommes de terre` une recherche.
_KNOWN_TLDS = frozenset(
    {
        "com", "org", "net", "fr", "io", "dev", "eu", "be", "ch", "ca", "uk",
        "de", "es", "it", "nl", "se", "no", "fi", "pl", "pt", "ru", "jp",
        "cn", "br", "au", "us", "info", "biz", "app", "xyz", "co", "me",
        "tv", "ai", "gg", "sh", "gov", "edu", "int", "mil", "wiki", "blog",
    }
)


class HistoryEntry:
    __slots__ = ("url", "title", "scroll")

    def __init__(self, url, title="", scroll=0):
        self.url = url
        self.title = title
        self.scroll = scroll

    def __repr__(self):
        return "HistoryEntry(%r, %r)" % (self.url, self.title)


class Page:
    """Document charge et mis en page."""

    __slots__ = ("url", "title", "document", "display_list", "response",
                 "error", "banner")

    def __init__(self, url="", title="", document=None, display_list=None,
                 response=None, error=None, banner=None):
        self.url = url
        self.title = title
        self.document = document
        self.display_list = display_list
        self.response = response
        self.error = error
        self.banner = banner if banner is not None else []

    @property
    def height(self):
        return self.display_list.height if self.display_list else 0

    def __repr__(self):
        return "Page(%r, %r)" % (self.url, self.title)


class Tab:
    """Un onglet : une pile d'historique et la page courante."""

    __slots__ = ("history", "position", "page", "scroll", "loading", "focused_field")

    def __init__(self):
        self.history = []
        self.position = -1
        self.page = None
        self.scroll = 0
        self.loading = False
        self.focused_field = None

    @property
    def url(self):
        if 0 <= self.position < len(self.history):
            return self.history[self.position].url
        return ""

    @property
    def title(self):
        if self.page is not None and self.page.title:
            return self.page.title
        if 0 <= self.position < len(self.history):
            return self.history[self.position].title or self.history[self.position].url
        return "Nouvel onglet"

    @property
    def can_go_back(self):
        return self.position > 0

    @property
    def can_go_forward(self):
        return self.position < len(self.history) - 1

    def push(self, url, title=""):
        """Empile une entree, en coupant la branche « avant » comme un navigateur."""
        if 0 <= self.position < len(self.history):
            self.history[self.position].scroll = self.scroll
        del self.history[self.position + 1 :]
        self.history.append(HistoryEntry(url, title))
        self.position = len(self.history) - 1
        self.scroll = 0

    def __repr__(self):
        return "Tab(%r, %d/%d)" % (self.url, self.position + 1, len(self.history))


class Browser:
    """Le navigateur : onglets, reglages, pipeline de chargement."""

    def __init__(self, width=1024, height=768, session=None, settings=None):
        self.settings = dict(DEFAULT_SETTINGS)
        if settings:
            self.settings.update(settings)
        self.settings["viewport-width"] = width
        self.settings["viewport-height"] = height

        self.session = session if session is not None else net.default_session()
        self.tabs = []
        self.active = 0
        self.global_history = []
        self.visited = set()
        self.status = ""
        self.on_change = None

    # -- Onglets -----------------------------------------------------------

    @property
    def tab(self):
        if not self.tabs:
            self.new_tab(load=False)
        return self.tabs[self.active]

    def new_tab(self, target=None, load=True, background=False):
        tab = Tab()
        self.tabs.append(tab)
        if not background:
            self.active = len(self.tabs) - 1
        if load:
            destination = target if target else self.settings["homepage"]
            index = len(self.tabs) - 1
            previous, self.active = self.active, index
            self.navigate(destination)
            if background:
                self.active = previous
        return tab

    def close_tab(self, index=None):
        if index is None:
            index = self.active
        if not (0 <= index < len(self.tabs)):
            return
        del self.tabs[index]
        if not self.tabs:
            self.new_tab(load=True)
            return
        self.active = min(self.active, len(self.tabs) - 1)

    def select_tab(self, index):
        if 0 <= index < len(self.tabs):
            self.active = index

    # -- Navigation --------------------------------------------------------

    def navigate(self, target, record=True):
        """Charge `target` dans l'onglet actif."""
        tab = self.tab
        resolved = self.resolve_input(target)
        tab.loading = True
        self._notify()
        try:
            page = self.load(resolved)
        finally:
            tab.loading = False

        tab.page = page
        if record:
            tab.push(page.url, page.title)
        elif 0 <= tab.position < len(tab.history):
            tab.history[tab.position].url = page.url
            tab.history[tab.position].title = page.title

        if page.error is None and not page.url.startswith("about:"):
            self.visited.add(page.url)
            self.global_history.append(HistoryEntry(page.url, page.title))
            del self.global_history[:-500]

        self.status = page.error or ""
        self._notify()
        return page

    def go_back(self):
        tab = self.tab
        if not tab.can_go_back:
            return None
        tab.history[tab.position].scroll = tab.scroll
        tab.position -= 1
        entry = tab.history[tab.position]
        tab.page = self.load(entry.url)
        tab.scroll = entry.scroll
        self._notify()
        return tab.page

    def go_forward(self):
        tab = self.tab
        if not tab.can_go_forward:
            return None
        tab.history[tab.position].scroll = tab.scroll
        tab.position += 1
        entry = tab.history[tab.position]
        tab.page = self.load(entry.url)
        tab.scroll = entry.scroll
        self._notify()
        return tab.page

    def reload(self, bypass_cache=True):
        tab = self.tab
        target = tab.url or self.settings["homepage"]
        if bypass_cache:
            self.session._cache.pop(target, None)
        tab.page = self.load(target)
        self._notify()
        return tab.page

    def home(self):
        return self.navigate(self.settings["homepage"])

    # -- Barre d'adresse ---------------------------------------------------

    def resolve_input(self, text):
        """Traduit une saisie en URL : adresse, chemin local ou recherche."""
        text = (text or "").strip()
        if not text:
            return "about:blank"

        lowered = text.lower()
        if lowered.startswith(("about:", "data:", "file:", "javascript:")):
            return text
        if lowered.startswith(("http://", "https://")):
            return text

        # Une saisie avec espace, ou sans point, est une recherche.
        has_space = " " in text
        head = text.split("/", 1)[0].split("?", 1)[0].split(":", 1)[0]
        looks_like_host = False
        if not has_space and "." in head:
            suffix = head.rsplit(".", 1)[-1].lower()
            looks_like_host = suffix in _KNOWN_TLDS or _is_ipv4(head)
        if head == "localhost" or _is_ipv4(head):
            # Les serveurs de developpement parlent rarement TLS.
            return "http://" + text

        if looks_like_host:
            return "https://" + text
        return self.search_url(text)

    def search_url(self, query):
        engine = self.settings.get("search-engine", "duckduckgo")
        template = SEARCH_ENGINES.get(engine, SEARCH_ENGINES["duckduckgo"])
        return template % urlmod.percent_encode(query)

    # -- Pipeline ----------------------------------------------------------

    def load(self, target):
        """Recupere, analyse, style et met en page une URL."""
        target = (target or "").strip()

        if target.startswith("about:") or target in ("", "blank"):
            html = pages.resolve(target, self)
            if html is None:
                html = pages.error(
                    "Page interne inconnue", "Aucune page ne repond a cette adresse.", target
                )
            return self._render(html, target, title_fallback=target)

        try:
            parsed = urlmod.parse(target)
        except ValueError as exc:
            html = pages.error("Adresse invalide", str(exc), target)
            return self._render(html, target)

        if parsed.scheme == "file":
            return self._load_file(parsed, target)

        if parsed.scheme not in ("http", "https"):
            html = pages.error(
                "Protocole non gere",
                "Nautile ne sait pas ouvrir les adresses en « %s: »." % parsed.scheme,
                target,
            )
            return self._render(html, target)

        response = self.session.fetch(parsed)
        if response.error is not None:
            html = pages.error(
                "Page inaccessible", response.error, target, details=response.trace
            )
            page = self._render(html, target)
            page.error = response.error
            return page

        if response.status >= 400:
            html = pages.error(
                "Erreur %d" % response.status,
                "Le serveur a refuse la requete.",
                response.url,
                details=response.trace,
            )
            page = self._render(html, response.url)
            page.error = "HTTP %d" % response.status
            return page

        if not response.is_html:
            return self._render_binary(response)

        page = self._render(response.text(), response.url, response=response)
        page.banner = response.trace
        return page

    def _load_file(self, parsed, target):
        path = urlmod.percent_decode(parsed.path)
        try:
            with open(path, "rb") as handle:
                raw = handle.read()
        except OSError as exc:
            html = pages.error("Fichier illisible", str(exc), target)
            page = self._render(html, target)
            page.error = str(exc)
            return page
        if path.lower().endswith((".html", ".htm", ".xhtml")):
            return self._render(htmlparser.decode_bytes(raw), target)
        text = htmlparser.decode_bytes(raw)
        html = "<html><head><title>%s</title></head><body><pre>%s</pre></body></html>" % (
            pages.escape(path),
            pages.escape(text),
        )
        return self._render(html, target)

    def _render_binary(self, response):
        ctype = response.content_type or "type inconnu"
        html = pages.error(
            "Contenu non affichable",
            "Cette ressource est de type %s (%d octets)." % (ctype, len(response.body)),
            response.url,
        )
        return self._render(html, response.url, response=response)

    def _render(self, html, url, response=None, title_fallback=""):
        """HTML -> DOM -> style -> layout. Le chemin unique de tout rendu."""
        width = self.settings["viewport-width"]
        height = self.settings["viewport-height"]
        viewport = (width, height)

        document = htmlparser.parse(html, url=url)
        sheets, links = stylemod.extract_stylesheets(document, viewport)

        if self.settings.get("load-stylesheets") and links and url.startswith(("http://", "https://")):
            for href in links[:12]:
                external = self._fetch_stylesheet(href, url, viewport)
                if external is not None:
                    sheets.append(external)

        context = stylemod.cssselect.MatchContext(visited=self.visited)
        engine = stylemod.StyleEngine(viewport=viewport, context=context)
        engine.add_stylesheet(stylemod.user_agent_stylesheet())
        for sheet in sheets:
            engine.add_stylesheet(sheet)
        engine.style_document(document)

        layout_engine = LayoutEngine(width=width, viewport=viewport)
        display = layout_engine.layout(document)
        display.url = url

        title = document.title or title_fallback or url
        display.title = title
        return Page(url=url, title=title, document=document,
                    display_list=display, response=response)

    def _fetch_stylesheet(self, href, base, viewport):
        try:
            absolute = urlmod.parse(href, base=base)
        except ValueError:
            return None
        response = self.session.fetch(absolute)
        if response.error is not None or not response.ok:
            return None
        from .css import parser as cssparser

        return cssparser.parse(response.text(), viewport=viewport)

    # -- Interaction -------------------------------------------------------

    def click(self, x, y):
        """Clic dans le document (coordonnees page). Renvoie l'action menee."""
        tab = self.tab
        if tab.page is None or tab.page.display_list is None:
            return None
        display = tab.page.display_list

        link = display.link_at(x, y)
        if link is not None:
            try:
                destination = urlmod.parse(link.href, base=tab.page.url)
            except ValueError:
                return None
            if link.href.lower().startswith("javascript:"):
                self.status = "JavaScript desactive"
                return None
            self.navigate(str(destination))
            return ("navigate", str(destination))

        field = display.field_at(x, y)
        if field is not None:
            tab.focused_field = field
            return ("focus", field)

        tab.focused_field = None
        return None

    def submit(self, field=None):
        """Soumet le formulaire du champ focalise (touche Entree)."""
        tab = self.tab
        field = field if field is not None else tab.focused_field
        if field is None or tab.page is None:
            return None

        pairs = list(field.hidden)
        if field.name:
            pairs.append((field.name, field.value))

        action = field.action or tab.page.url
        try:
            destination = urlmod.parse(action, base=tab.page.url)
        except ValueError:
            return None

        query = urlmod.encode_form(pairs)
        if field.method == "post":
            # POST non implemente cote transport : on degrade en GET plutot
            # que d'echouer silencieusement.
            self.status = "formulaire POST envoye en GET"
        destination = urlmod.URL(
            destination.scheme, destination.host, destination.port,
            destination.path, query, "",
        )
        self.navigate(str(destination))
        return ("navigate", str(destination))

    def scroll_by(self, delta):
        tab = self.tab
        height = tab.page.height if tab.page else 0
        viewport = self.settings["viewport-height"]
        maximum = max(0, height - viewport)
        tab.scroll = max(0, min(maximum, tab.scroll + delta))
        return tab.scroll

    def resize(self, width, height):
        """Change le viewport et remet en page sans refaire de requete."""
        self.settings["viewport-width"] = max(200, int(width))
        self.settings["viewport-height"] = max(120, int(height))
        tab = self.tab
        if tab.page is not None and tab.page.document is not None:
            engine = LayoutEngine(
                width=self.settings["viewport-width"],
                viewport=(self.settings["viewport-width"], self.settings["viewport-height"]),
            )
            tab.page.display_list = engine.layout(tab.page.document)
            tab.page.display_list.url = tab.page.url
            tab.page.display_list.title = tab.page.title
        self._notify()

    def _notify(self):
        if self.on_change is not None:
            self.on_change(self)


def _is_ipv4(text):
    parts = text.split(".")
    if len(parts) != 4:
        return False
    for part in parts:
        if not part.isdigit() or not 0 <= int(part) <= 255:
            return False
    return True
