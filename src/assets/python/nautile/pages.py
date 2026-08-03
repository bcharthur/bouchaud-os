"""Pages internes `about:` et pages d'erreur.

Elles sont produites en HTML puis passees au meme pipeline que le reste : un
seul moteur de rendu, aucune voie speciale. C'est ce qui garantit qu'une page
interne teste reellement le navigateur.
"""

from . import version

_SHELL = """<!doctype html>
<html><head><title>%(title)s</title><style>
  body { font-family: sans-serif; margin: 0; background: #f6f7f9; color: #202124 }
  .wrap { margin: 40px; max-width: 720px }
  h1 { font-size: 26px; margin-top: 0; margin-bottom: 6px }
  h2 { font-size: 16px; margin-top: 26px; margin-bottom: 8px; color: #5f6368 }
  p { margin-top: 8px; margin-bottom: 8px; line-height: 1.6 }
  .card { background: #ffffff; border: 1px solid #dfe3e8; padding: 16px; margin-top: 12px }
  .muted { color: #5f6368; font-size: 14px }
  code { background: #eef0f3; padding: 2px; font-family: monospace }
  a { color: #1a56c4 }
  table { border-spacing: 0 }
  td { padding: 4px }
  .k { color: #5f6368; width: 190px }
</style></head><body><div class="wrap">%(body)s</div></body></html>
"""


def _page(title, body):
    return _SHELL % {"title": title, "body": body}


def blank():
    return _page("Nouvel onglet", '<h1>Nouvel onglet</h1><p class="muted">Saisis une adresse ou une recherche.</p>')


def home():
    links = [
        ("Wikipedia", "https://fr.wikipedia.org/"),
        ("Hacker News", "https://news.ycombinator.com/"),
        ("Python", "https://www.python.org/"),
        ("MDN", "https://developer.mozilla.org/fr/"),
        ("example.com", "https://example.com/"),
    ]
    cards = "".join(
        '<p><a href="%s">%s</a> <span class="muted">%s</span></p>' % (url, name, url)
        for name, url in links
    )
    return _page(
        "Nautile",
        '<h1>Nautile</h1>'
        '<p class="muted">Navigateur Web ecrit en Python, moteur de rendu compris.</p>'
        '<h2>Raccourcis</h2><div class="card">%s</div>'
        '<h2>Pages internes</h2><div class="card">'
        '<p><a href="about:version">about:version</a> — version et moteur</p>'
        '<p><a href="about:config">about:config</a> — reglages effectifs</p>'
        '<p><a href="about:cache">about:cache</a> — contenu du cache</p>'
        '</div>' % cards,
    )


def version_page():
    rows = [
        ("Nautile", version.VERSION),
        ("Nom de code", version.CODENAME),
        ("Moteur de rendu", version.ENGINE),
        ("User-Agent", version.user_agent()),
        ("Plateforme", version.platform_name()),
        ("Python", version.python_version()),
    ]
    body = "".join(
        '<tr><td class="k">%s</td><td><code>%s</code></td></tr>' % row for row in rows
    )
    return _page(
        "A propos de Nautile",
        '<h1>Nautile</h1><p class="muted">%s</p>'
        '<div class="card"><table>%s</table></div>'
        '<h2>Pipeline</h2><div class="card"><p class="muted">'
        'URL &rarr; reseau &rarr; HTML &rarr; DOM &rarr; CSS &rarr; style &rarr; '
        'layout &rarr; liste d\'affichage &rarr; backend de rendu</p></div>'
        % (version.TAGLINE, body),
    )


def config_page(settings):
    rows = "".join(
        '<tr><td class="k">%s</td><td><code>%s</code></td></tr>' % (key, value)
        for key, value in sorted(settings.items())
    )
    return _page(
        "Configuration",
        '<h1>Configuration</h1><p class="muted">Reglages effectifs de cette session.</p>'
        '<div class="card"><table>%s</table></div>' % rows,
    )


def cache_page(entries):
    if not entries:
        rows = '<p class="muted">Le cache est vide.</p>'
    else:
        rows = "".join(
            '<p><code>%s</code> <span class="muted">%d octets</span></p>' % (url, size)
            for url, size in entries
        )
    return _page(
        "Cache",
        '<h1>Cache</h1><p class="muted">%d ressource(s) en memoire.</p>'
        '<div class="card">%s</div>' % (len(entries), rows),
    )


def history_page(entries):
    if not entries:
        rows = '<p class="muted">Aucune page visitee.</p>'
    else:
        rows = "".join(
            '<p><a href="%s">%s</a><br><span class="muted">%s</span></p>'
            % (url, escape(title or url), escape(url))
            for url, title in entries
        )
    return _page("Historique", '<h1>Historique</h1><div class="card">%s</div>' % rows)


def error(title, message, url="", details=()):
    extra = ""
    if details:
        extra = '<h2>Details</h2><div class="card">%s</div>' % "".join(
            '<p class="muted"><code>%s</code></p>' % escape(str(line)) for line in details
        )
    return _page(
        title,
        '<h1>%s</h1><p>%s</p>%s%s'
        % (
            escape(title),
            escape(message),
            '<p class="muted"><code>%s</code></p>' % escape(url) if url else "",
            extra,
        ),
    )


def escape(text):
    """Echappe pour insertion dans du HTML."""
    return (
        str(text)
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
    )


def resolve(target, browser=None):
    """Rend une page `about:`. Renvoie le HTML, ou None si inconnue."""
    name = target.strip().lower()
    if name.startswith("about:"):
        name = name[6:]
    if name in ("", "blank", "newtab"):
        return blank()
    if name in ("home", "start", "nautile"):
        return home()
    if name in ("version", "about"):
        return version_page()
    if name == "config":
        settings = browser.settings if browser is not None else {}
        return config_page(settings)
    if name == "cache":
        entries = []
        if browser is not None:
            for key, response in browser.session._cache.items():
                entries.append((key, len(response.body)))
        return cache_page(entries)
    if name == "history":
        entries = []
        if browser is not None:
            entries = [(entry.url, entry.title) for entry in browser.global_history]
        return history_page(entries)
    return None
