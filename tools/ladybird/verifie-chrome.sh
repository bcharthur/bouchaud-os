#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."
RACINE=$(pwd)
SRC="$RACINE/third_party/ladybird-browser-src"
[ -f "$SRC/Services/WebContent/BouchaudChrome.h" ] || { echo "V16: arbre Ladybird prepare absent" >&2; exit 1; }
python3 tools/ladybird/chrome/modernise-v15.py "$SRC"
python3 tools/ladybird/prepare-v16-fonts.py "$SRC"
python3 tools/ladybird/chrome/modernise-v16.py "$SRC"
CHROME="$SRC/Services/WebContent/BouchaudChrome.h"
CMAKE="$SRC/Services/WebContent/CMakeLists.txt"
grep -q 'BOUCHAUD_CHROME_V15_REAL_TEXT_SVG_LOADING' "$CHROME"
grep -q 'BOUCHAUD_CHROME_V16_FONTCONFIG_TYPEFACE' "$CHROME"
grep -Fq 'mask & (Modificateur::Alt | Modificateur::AltGr)' "$CHROME"
if grep -Fq 'KeyModifier::Mod_AltGr' "$CHROME"; then
    echo 'chrome Bouchaud: API Ladybird inexistante KeyModifier::Mod_AltGr' >&2
    exit 1
fi
grep -q 'BouchaudChromeV15Assets::STOP' "$CHROME"
# Le texte d'interface passe par un point unique, et c'est ce point que V15
# modernise. L'ancre peut correspondre sans que le corps soit celui qu'on
# croit : verifier le RESULTAT et pas seulement l'ancre est ce qui distingue
# « la substitution a eu lieu » de « le chrome est bien en DejaVu ».
grep -Fq 'if (draw_browser_text(canvas, x, y, texte, couleur, largeur_max))' "$CHROME"
grep -q 'target_link_libraries(webcontentservice PRIVATE skia)' "$CMAKE"
grep -q 'FONTS_V16_FORCE_FONTCONFIG' "$SRC/Services/WebContent/main.cpp"
grep -q 'BOUCHAUD_V16_DEJAVU_GENERIC' "$SRC/Libraries/LibWeb/Platform/FontPlugin.cpp"
grep -q 'BOUCHAUD_V16_PATH_FONT_ALIAS' "$SRC/Libraries/LibGfx/Font/PathFontProvider.cpp"
# Les greps ci-dessus disent que les MODERNISATIONS ont porte. Ils ne disent
# rien de ce que le chrome compile : c'est `webcontentservice` qui l'apprend,
# parmi les derniers objets de la construction, six minutes plus tard sur un
# cache chaud et vingt sur un cache froid. L'analyse ci-dessous fait le meme
# travail en une trentaine de secondes, sur ce meme arbre.
#
# Elle ne peut pas faire echouer la barriere pour une raison d'outillage :
# sans clang, elle le dit et passe.
./tools/ladybird/verifie-syntaxe-chrome.sh "$SRC"

printf '\033[32m%s\033[0m\n' 'chrome V16: DejaVu/FreeType + SVG + loading indicator OK'
