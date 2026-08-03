"""Point d'entree en ligne de commande : `python -m nautile`."""

import sys

from . import backends, version
from .browser import Browser

USAGE = """nautile — navigateur Web ecrit en Python

usage: python -m nautile [URL] [options]

options:
  --backend NOM     qt | tk | bouchaud | text   (defaut : le meilleur disponible)
  --engine NOM      nautile | webengine         (backend qt uniquement)
  --dump            rend la page en texte sur la sortie standard, puis quitte
  --outline         liste les items de la liste d'affichage, puis quitte
  --width N         largeur du viewport en pixels (defaut 1024)
  --height N        hauteur du viewport en pixels (defaut 768)
  --columns N       largeur du rendu texte en caracteres (defaut 100)
  --search MOTEUR   duckduckgo | google | bing | wikipedia
  --list-backends   affiche les backends disponibles, puis quitte
  --version         affiche la version, puis quitte
  -h, --help        affiche cette aide

exemples:
  python -m nautile                          # interface graphique, page d'accueil
  python -m nautile https://example.com      # ouvre une page
  python -m nautile example.com --dump       # rendu texte, sans interface
  python -m nautile "chat botte" --dump      # recherche depuis la barre d'adresse
"""


def parse_args(argv):
    options = {
        "url": None,
        "backend": None,
        "engine": "nautile",
        "dump": False,
        "outline": False,
        "width": 1024,
        "height": 768,
        "columns": 100,
        "search": None,
    }
    index = 0
    while index < len(argv):
        arg = argv[index]
        if arg in ("-h", "--help"):
            print(USAGE)
            raise SystemExit(0)
        if arg == "--version":
            print("nautile %s — %s" % (version.VERSION, version.ENGINE))
            raise SystemExit(0)
        if arg == "--list-backends":
            print(" ".join(backends.available_backends()))
            raise SystemExit(0)
        if arg == "--dump":
            options["dump"] = True
        elif arg == "--outline":
            options["outline"] = True
        elif arg in ("--backend", "--engine", "--width", "--height", "--columns", "--search"):
            index += 1
            if index >= len(argv):
                raise SystemExit("nautile: %s attend une valeur" % arg)
            key = arg[2:]
            value = argv[index]
            options[key] = int(value) if key in ("width", "height", "columns") else value
        elif arg.startswith("-"):
            raise SystemExit("nautile: option inconnue %s (voir --help)" % arg)
        elif options["url"] is None:
            options["url"] = arg
        else:
            # Une saisie non quotee : on recolle les mots en une recherche.
            options["url"] += " " + arg
        index += 1
    return options


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    options = parse_args(argv)
    target = options["url"] or "about:home"

    if options["dump"] or options["outline"]:
        return _render_text(options, target)

    choice = backends.pick(options["backend"])

    if choice == "qt":
        from .backends import qt

        return qt.run(target, engine=options["engine"])
    if choice == "tk":
        from .backends import tk

        return tk.run(target)
    if choice == "bouchaud":
        from .backends import bouchaud

        return bouchaud.run(target, options["width"], options["height"])

    return _render_text(options, target)


def _render_text(options, target):
    from .backends.text import dump, outline

    # La grille texte fait 8 px par colonne : mettre la page en page sur une
    # autre largeur ferait deborder les lignes hors de la grille.
    width = options["columns"] * 8 if not options["outline"] else options["width"]
    browser = Browser(width=width, height=options["height"])
    if options["search"]:
        browser.settings["search-engine"] = options["search"]

    page = browser.navigate(target)
    if options["outline"]:
        print(outline(page.display_list))
    else:
        print(dump(page, columns=options["columns"]))
    return 1 if page.error else 0


if __name__ == "__main__":
    sys.exit(main())
