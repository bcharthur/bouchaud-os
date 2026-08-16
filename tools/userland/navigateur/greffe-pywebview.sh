#!/bin/bash
# Installe pywebview et son moteur Bouchaud OS dans l'arborescence du disque.
#
#   ./greffe-pywebview.sh <destination>
#
# `destination` recoit `pywebview`, `bottle`, `proxy_tools` et
# `typing_extensions` — les trois dernieres sont les dependances declarees de
# pywebview, toutes en Python pur, donc utilisables telles quelles.
#
# ## Ce qui est greffe
#
# pywebview choisit son moteur d'affichage dans `webview/guilib.py`, parmi une
# liste fermee (`qt`, `gtk`, `cocoa`, `edgechromium`...). On y ajoute
# `bouchaud`, et on le fait essayer en premier sous Linux. C'est la seule
# modification apportee a la bibliotheque : le reste — l'API publique, le
# serveur HTTP interne, le modele de fenetre — est utilise tel quel.
#
# Le moteur lui-meme (`webview/platforms/bouchaud.py`) vient de
# `webview_bouchaud.py`, a cote de ce script.

set -euo pipefail
cd "$(dirname "$0")"
SOURCE=$PWD

DEST=${1:?usage: greffe-pywebview.sh <destination>}
case "$DEST" in /*) ;; *) DEST=$PWD/$DEST ;; esac
CHANTIER=${CHANTIER:-$SOURCE/../build-navigateur/pywebview}
case "$CHANTIER" in /*) ;; *) CHANTIER=$SOURCE/$CHANTIER ;; esac

PYWEBVIEW_VER=${PYWEBVIEW_VER:-6.2.1}
BOTTLE_VER=${BOTTLE_VER:-0.13.4}
PROXYTOOLS_VER=${PROXYTOOLS_VER:-0.1.0}
TYPINGEXT_VER=${TYPINGEXT_VER:-4.16.0}

mkdir -p "$CHANTIER"
cd "$CHANTIER"

telecharge() { # telecharge <paquet> <version>
    local nom=$1
    local version=$2
    local archive="$nom.tar.gz"
    local meta="$nom-$version.pypi.json"
    local tmp="$archive.tmp"

    # Ne jamais reutiliser silencieusement une archive corrompue issue d'un
    # ancien run interrompu.
    if [ -f "$archive" ] && tar -tzf "$archive" >/dev/null 2>&1; then
        return 0
    fi
    rm -f "$archive" "$tmp" "$meta"

    echo "  PyPI: $nom==$version"
    # Le run #90 recevait parfois une reponse vide de PyPI : le pipe envoyait
    # alors directement EOF a json.load() et produisait un JSONDecodeError peu
    # parlant. Telecharger d'abord le metadata dans un fichier, avec retries,
    # puis seulement le parser rend la panne reseau explicite et recuperable.
    curl --fail --silent --show-error --location \
         --retry 5 --retry-all-errors --retry-delay 2 \
         --connect-timeout 10 --max-time 90 \
         -o "$meta" "https://pypi.org/pypi/$nom/$version/json"
    [ -s "$meta" ] || {
        echo "PyPI: metadata vide pour $nom==$version" >&2
        return 1
    }

    local url
    url=$(python3 - "$meta" "$nom" "$version" <<'PY'
import json
import sys

path, name, version = sys.argv[1:]
try:
    with open(path, "rb") as stream:
        data = json.load(stream)
except (OSError, json.JSONDecodeError) as exc:
    raise SystemExit(f"PyPI: metadata invalide pour {name}=={version}: {exc}")

if data.get("info", {}).get("version") != version:
    raise SystemExit(
        f"PyPI: version inattendue pour {name}: "
        f"{data.get('info', {}).get('version')!r}, attendu {version!r}"
    )

sdists = [entry.get("url") for entry in data.get("urls", [])
          if entry.get("packagetype") == "sdist" and entry.get("url")]
if not sdists:
    raise SystemExit(f"PyPI: aucun sdist pour {name}=={version}")
print(sdists[0])
PY
    )

    curl --fail --silent --show-error --location \
         --retry 5 --retry-all-errors --retry-delay 2 \
         --connect-timeout 10 --max-time 300 \
         -o "$tmp" "$url"
    tar -tzf "$tmp" >/dev/null || {
        echo "PyPI: archive invalide pour $nom==$version" >&2
        rm -f "$tmp"
        return 1
    }
    mv "$tmp" "$archive"
    rm -f "$meta"
}

telecharge pywebview "$PYWEBVIEW_VER"
telecharge bottle "$BOTTLE_VER"
telecharge proxy_tools "$PROXYTOOLS_VER"
telecharge typing_extensions "$TYPINGEXT_VER"

rm -rf extrait && mkdir extrait
for paquet in pywebview bottle proxy_tools typing_extensions; do
    tar xf "$paquet.tar.gz" -C extrait
done

mkdir -p "$DEST"
cp -r "extrait/pywebview-$PYWEBVIEW_VER/webview" "$DEST/"
cp "extrait/bottle-$BOTTLE_VER/bottle.py" "$DEST/"
cp "extrait/proxy_tools-$PROXYTOOLS_VER/proxy_tools/__init__.py" \
   "$DEST/proxy_tools_pkg.py" 2>/dev/null || true
mkdir -p "$DEST/proxy_tools"
cp "extrait/proxy_tools-$PROXYTOOLS_VER/proxy_tools/__init__.py" "$DEST/proxy_tools/"
rm -f "$DEST/proxy_tools_pkg.py"
cp "extrait/typing_extensions-$TYPINGEXT_VER/src/typing_extensions.py" "$DEST/" \
   2>/dev/null || cp "extrait/typing_extensions-$TYPINGEXT_VER/typing_extensions.py" "$DEST/"

# Le moteur Bouchaud OS.
cp "$SOURCE/webview_bouchaud.py" "$DEST/webview/platforms/bouchaud.py"

# Le schema `bo:` des pages internes du moteur (voir greffe_util.py).
python3 "$SOURCE/greffe_util.py" "$DEST/webview/util.py"

# Greffe dans le selecteur de moteur.
python3 - "$DEST/webview/guilib.py" <<'PY'
import sys

chemin = sys.argv[1]
source = open(chemin).read()

# 1. Le type de moteur devient une valeur acceptee, y compris par
#    PYWEBVIEW_GUI=bouchaud.
avant = ("GUIType: TypeAlias = Literal['qt', 'gtk', 'cef', 'mshtml', "
         "'edgechromium', 'android', 'cocoa']")
apres = ("GUIType: TypeAlias = Literal['bouchaud', 'qt', 'gtk', 'cef', 'mshtml', "
         "'edgechromium', 'android', 'cocoa']")
assert avant in source, "signature de GUIType inattendue"
source = source.replace(avant, apres)

# 2. La fonction d'import du moteur.
avant = "    def import_gtk():"
apres = '''    def import_bouchaud():
        """Moteur natif de Bouchaud OS : moteur web maison sur toile Qt."""
        global guilib

        try:
            import webview.platforms.bouchaud as guilib

            logger.debug('Using Bouchaud OS')
            return True
        except ImportError:
            return False

    def import_gtk():'''
assert avant in source, "import_gtk introuvable"
source = source.replace(avant, apres, 1)

# 3. Sous Linux, on l'essaie avant les autres : c'est le seul qui puisse
#    fonctionner ici, et son echec laisse la liste habituelle se derouler.
remplace = 0
for avant, apres in (
    ("        if forced_gui == 'qt':\n            guis = [import_qt, import_gtk]\n"
     "        else:\n            guis = [import_gtk, import_qt]",
     "        if forced_gui == 'qt':\n            guis = [import_qt, import_gtk]\n"
     "        elif forced_gui == 'gtk':\n            guis = [import_gtk, import_qt]\n"
     "        else:\n            guis = [import_bouchaud, import_gtk, import_qt]"),
):
    if avant in source:
        source = source.replace(avant, apres, 1)
        remplace += 1
assert remplace == 1, "liste des moteurs Linux inattendue"

open(chemin, 'w').write(source)
print("  guilib.py : moteur « bouchaud » greffe")
PY

find "$DEST" -name '__pycache__' -type d -exec rm -rf {} + 2>/dev/null || true
find "$DEST" -name 'tests' -type d -exec rm -rf {} + 2>/dev/null || true

echo "  pywebview $PYWEBVIEW_VER installe dans $DEST"
