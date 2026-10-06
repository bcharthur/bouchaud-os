#!/bin/bash
# Verifie l'arbre Ladybird PREPARE avant d'engager le build (BOUCHAUD_UI_V1).
#
# Ce script ne modifie rien : les modernisations V15/V16 du chrome sont dans
# sa source (`tools/ladybird/chrome/BouchaudChrome.h`) et la correction des
# polices de LibWeb est un preparateur de la chaine (`prepare-v16-fonts.py`).
# Il controle le RESULTAT de la chaine :
#
#   1. le frontend UI/Bouchaud est installe et construit ;
#   2. WebContent ne porte plus aucun chrome ni pont M8/M9/M11 ;
#   3. WebContent garde le Compositor upstream (aucun retour a la capture) ;
#   4. le chrome rend son texte par Skia/FontConfig (V15/V16) ;
#   5. le chrome se compile (analyse syntaxique, `verifie-syntaxe-chrome.sh`).
set -euo pipefail
cd "$(dirname "$0")/../.."
SRC="${1:-third_party/ladybird-browser-src}"
UI="$SRC/UI/Bouchaud"
CHROME="$UI/BouchaudChrome.h"
WC="$SRC/Services/WebContent"

faute() { echo "verifie-chrome: $*" >&2; exit 1; }

[ -f "$CHROME" ] || faute "UI/Bouchaud absent ($CHROME) -- prepare-ui-bouchaud.py n'a pas tourne"
for f in main.cpp Application.cpp BouchaudWebView.cpp BrowserWindow.cpp CMakeLists.txt; do
    [ -f "$UI/$f" ] || faute "UI/Bouchaud/$f absent"
done
grep -q 'add_subdirectory(${LADYBIRD_SOURCE_DIR}/UI/Bouchaud' "$SRC/Services/CMakeLists.txt" \
    || faute "UI/Bouchaud n'est pas construit"
[ ! -e "$SRC/Services/BouchaudBrowserHost" ] || faute "ancien hote Services/BouchaudBrowserHost encore present"

# 2. WebContent : du contenu, rien d'autre.
if ls "$WC"/Bouchaud*.h >/dev/null 2>&1; then
    faute "en-tete du chrome dans WebContent : $(ls "$WC"/Bouchaud*.h | xargs -n1 basename | tr '\n' ' ')"
fi
for interdit in BouchaudChrome bouchaud_m11_start bouchaud_m9_start bouchaud_m8_start BO_GUI_FD BO_SURFACE_FD \
    bouchaud_inject_ BOUCHAUD_WEBCONTENT_FD bouchaud_hote_local page_did_take_screenshot.*BouchaudChrome; do
    if grep -rqE "$interdit" "$WC"; then
        faute "WebContent porte encore $interdit"
    fi
done
if grep -rqE 'bouchaud_(schedule|request)_interactive_frame_capture|bouchaud_last_frame_damage' "$SRC/Libraries/LibWeb"; then
    faute "LibWeb porte encore la capture interactive (prepare-repaint)"
fi

# 3. Le Compositor upstream.
grep -q 'virtual bool supports_compositor() const override { return true; }' "$WC/PageClient.h" \
    || faute "PageClient::supports_compositor n'est plus celui d'upstream"
grep -q 'did_accept_presented_backing_store' "$UI/BouchaudWebView.cpp" \
    || faute "la vue ne recoit plus la trame presentee"

# 4. Texte Skia + FontConfig (V15/V16), dans la source du chrome.
grep -q 'BOUCHAUD_CHROME_V15_REAL_TEXT_SVG_LOADING' "$CHROME" || faute "texte Skia V15 absent du chrome"
grep -q 'BOUCHAUD_CHROME_V16_FONTCONFIG_TYPEFACE' "$CHROME" || faute "typeface FontConfig V16 absente du chrome"
grep -Fq 'mask & (Modificateur::Alt | Modificateur::AltGr)' "$CHROME" || faute "AltGr non traduit"
if grep -Fq 'KeyModifier::Mod_AltGr' "$CHROME"; then
    faute "API Ladybird inexistante KeyModifier::Mod_AltGr"
fi
grep -q 'BouchaudChromeV15Assets::STOP' "$CHROME" || faute "icones V15 absentes"
grep -Fq 'if (draw_browser_text(canvas, x, y, texte, couleur, largeur_max))' "$CHROME" \
    || faute "draw_ui_text ne passe plus par le rendu Skia"
grep -q 'FONTS_V16_FORCE_FONTCONFIG' "$WC/main.cpp" || faute "prepare-v16-fonts n'a pas porte sur WebContent"
grep -q 'BOUCHAUD_V16_DEJAVU_GENERIC' "$SRC/Libraries/LibWeb/Platform/FontPlugin.cpp" || faute "alias DejaVu absent"
grep -q 'BOUCHAUD_V16_PATH_FONT_ALIAS' "$SRC/Libraries/LibGfx/Font/PathFontProvider.cpp" || faute "alias PathFont absent"
grep -q 'skia' "$UI/CMakeLists.txt" || faute "UI/Bouchaud ne lie pas Skia"

# 5. Le chrome se compile.
./tools/ladybird/verifie-syntaxe-chrome.sh "$SRC"

printf '\033[32m%s\033[0m\n' 'UI/Bouchaud : frontend installe, WebContent sans chrome, Compositor upstream, chrome V16 OK'
