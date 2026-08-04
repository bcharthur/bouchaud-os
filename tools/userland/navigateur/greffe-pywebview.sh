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

set -e
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

telecharge() { # telecharge <paquet>
    local nom=$1
    [ -f "$nom.tar.gz" ] && return 0
    local url
    url=$(curl -sf --max-time 30 "https://pypi.org/pypi/$nom/json" | python3 -c "
import json, sys
d = json.load(sys.stdin)
sdist = [f['url'] for f in d['urls'] if f['packagetype'] == 'sdist']
wheel = [f['url'] for f in d['urls'] if f['packagetype'] == 'bdist_wheel']
print((sdist or wheel)[0])")
    curl -sfL --max-time 300 -o "$nom.tar.gz" "$url"
}

for paquet in pywebview bottle proxy_tools typing_extensions; do
    telecharge "$paquet"
done

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
