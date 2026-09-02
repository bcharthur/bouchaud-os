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
grep -q 'BouchaudChromeV15Assets::STOP' "$CHROME"
grep -q 'target_link_libraries(webcontentservice PRIVATE skia)' "$CMAKE"
grep -q 'FONTS_V16_FORCE_FONTCONFIG' "$SRC/Services/WebContent/main.cpp"
grep -q 'BOUCHAUD_V16_DEJAVU_GENERIC' "$SRC/Libraries/LibWeb/Platform/FontPlugin.cpp"
grep -q 'BOUCHAUD_V16_PATH_FONT_ALIAS' "$SRC/Libraries/LibGfx/Font/PathFontProvider.cpp"
printf '\033[32m%s\033[0m\n' 'chrome V16: DejaVu/FreeType + SVG + loading indicator OK'
