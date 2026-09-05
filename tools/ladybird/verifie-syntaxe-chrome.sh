#!/bin/bash
# Analyse syntaxique et semantique du chrome, sans construire Ladybird.
#
# ## Ce qu'il attrape, et ce que cela coute autrement
#
# Le chrome est un en-tete de plusieurs milliers de lignes, compile par
# `webcontentservice`, c'est-a-dire par l'un des DERNIERS objets de la
# construction. Une faute de frappe -- un argument oublie, un nom mal ecrit, une
# fonction employee avant d'etre declaree -- ne se voit donc qu'apres avoir
# reconstruit tout le reste : six minutes sur un cache chaud, vingt sur un cache
# froid, et un aller-retour de CI a chaque fois.
#
# Ce script fait la meme analyse en une trentaine de secondes.
#
# ## Comment il s'affranchit de la construction
#
# Ladybird GENERE trois familles d'en-tetes que le compilateur exige et dont le
# chrome ne se sert pas : les `Export.h` de chaque bibliotheque (qui definissent
# des macros de visibilite), `AK/Debug.h` (des drapeaux de journalisation), et
# les en-tetes de Skia (fournis par vcpkg). Aucune ne change la SEMANTIQUE de ce
# qu'on verifie ici.
#
# Elles sont donc remplacees par des bouchons fabriques a la volee -- macros
# vides, drapeaux a zero, declarations Skia reduites aux membres que le chrome
# appelle. Ce que ce script ne peut pas prouver est exactement ce que ces
# bouchons cachent : que Skia ait bien ces signatures-la. Tout le reste -- la
# totalite du chrome, de LibGfx, de LibWeb, d'AK -- est le vrai code.
#
#   ./tools/ladybird/verifie-syntaxe-chrome.sh [arbre-prepare]
#
# Sans argument, il prend `third_party/ladybird-browser-src`, l'arbre que
# `browser-upstream.sh` prepare. L'arbre doit avoir recu les preparateurs ET les
# modernisations V15/V16 : c'est ce texte-la qui sera compile.
set -eu
cd "$(dirname "$0")/../.."

SRC=${1:-third_party/ladybird-browser-src}
CHROME="$SRC/Services/WebContent/BouchaudChrome.h"

if [ ! -f "$CHROME" ]; then
    echo "syntaxe chrome : arbre prepare absent ($CHROME)" >&2
    echo "                 lancer ./tools/ladybird/browser-upstream.sh, ou passer un chemin" >&2
    exit 1
fi

CLANG=""
for candidat in clang++-20 clang++-19 clang++-18 clang++; do
    if command -v "$candidat" >/dev/null 2>&1; then CLANG=$candidat; break; fi
done
if [ -z "$CLANG" ]; then
    # Un compilateur absent n'est pas une faute du code : on le DIT et on passe.
    # `g++` ne convient pas -- AK emploie « deducing this », que GCC 13 ne
    # connait pas --, et faire echouer la barriere sur l'outillage de la machine
    # apprendrait a l'ignorer.
    echo "syntaxe chrome : aucun clang++ trouve, verification passee"
    exit 0
fi

BOUCHONS=$(mktemp -d)
trap 'rm -rf "$BOUCHONS"' EXIT

APIS="CORE GFX WEB IPC JS GC URL UNICODE REGEX TEXTCODEC CRYPTO TLS HTTP
COMPRESS WASM WEBVIEW SYNTAX DIFF LINE MAIN MEDIA RIFF THREADING XML DEVTOOLS
AUDIO FILESYSTEM WEBSOCKET REQUESTS SQL TEST IMAGEDECODERCLIENT WEBCONTENT PDF"

bouchon_export() {
    mkdir -p "$BOUCHONS/$(dirname "$1")"
    {
        echo '#pragma once'
        for api in $APIS; do
            printf '#ifndef %s_API\n#define %s_API\n#endif\n' "$api" "$api"
        done
    } > "$BOUCHONS/$1"
}

# `AK/Debug.h` est genere a partir d'une liste de drapeaux. Les DECOUVRIR dans
# la source vaut mieux que de les recopier : un drapeau ajoute chez upstream ne
# doit pas faire echouer cette verification-ci.
mkdir -p "$BOUCHONS/AK"
{
    echo '#pragma once'
    grep -rhoE 'dbgln_if\([A-Z0-9_]+_DEBUG' "$SRC/AK" "$SRC/Libraries" "$SRC/Services" 2>/dev/null \
        | sed 's/dbgln_if(//' | sort -u | sed 's/^/#define /; s/$/ 0/'
} > "$BOUCHONS/AK/Debug.h"

mkdir -p "$BOUCHONS/core" "$BOUCHONS/ports"
cat > "$BOUCHONS/core/SkTypes.h" <<'EOF'
#pragma once
#include <cstddef>
#include <cstdint>
template<typename T>
class sk_sp {
public:
    sk_sp() = default;
    sk_sp(T* p) : m_p(p) { }
    explicit operator bool() const { return m_p != nullptr; }
    T* operator->() const { return m_p; }
    T& operator*() const { return *m_p; }
    T* get() const { return m_p; }
private:
    T* m_p { nullptr };
};
using SkColor = uint32_t;
inline SkColor SkColorSetARGB(uint32_t a, uint32_t r, uint32_t g, uint32_t b)
{
    return (a << 24) | (r << 16) | (g << 8) | b;
}
enum SkColorType { kBGRA_8888_SkColorType };
enum SkAlphaType { kOpaque_SkAlphaType };
enum class SkTextEncoding { kUTF8 };
EOF
cat > "$BOUCHONS/core/SkImageInfo.h" <<'EOF'
#pragma once
#include <core/SkTypes.h>
struct SkImageInfo {
    static SkImageInfo Make(int, int, SkColorType, SkAlphaType) { return {}; }
};
EOF
cat > "$BOUCHONS/core/SkBitmap.h" <<'EOF'
#pragma once
#include <core/SkImageInfo.h>
class SkBitmap {
public:
    bool installPixels(SkImageInfo const&, void*, size_t) { return true; }
};
EOF
cat > "$BOUCHONS/core/SkRect.h" <<'EOF'
#pragma once
#include <core/SkTypes.h>
struct SkRect {
    static SkRect MakeXYWH(float, float, float, float) { return {}; }
};
EOF
cat > "$BOUCHONS/core/SkFontStyle.h" <<'EOF'
#pragma once
class SkFontStyle {
public:
    SkFontStyle() = default;
};
EOF
cat > "$BOUCHONS/core/SkTypeface.h" <<'EOF'
#pragma once
#include <core/SkFontStyle.h>
#include <core/SkTypes.h>
class SkTypeface {
public:
    static sk_sp<SkTypeface> MakeFromName(char const*, SkFontStyle) { return {}; }
};
EOF
cat > "$BOUCHONS/core/SkFontMgr.h" <<'EOF'
#pragma once
#include <core/SkTypeface.h>
class SkFontScanner;
class SkFontMgr {
public:
    sk_sp<SkTypeface> matchFamilyStyle(char const*, SkFontStyle) { return {}; }
};
EOF
cat > "$BOUCHONS/core/SkFont.h" <<'EOF'
#pragma once
#include <core/SkTypeface.h>
class SkFont {
public:
    enum class Edging { kAntiAlias };
    SkFont(sk_sp<SkTypeface>, float) { }
    void setEdging(Edging) { }
    void setSubpixel(bool) { }
};
EOF
cat > "$BOUCHONS/core/SkPaint.h" <<'EOF'
#pragma once
#include <core/SkTypes.h>
class SkPaint {
public:
    void setAntiAlias(bool) { }
    void setColor(SkColor) { }
};
EOF
cat > "$BOUCHONS/core/SkCanvas.h" <<'EOF'
#pragma once
#include <core/SkBitmap.h>
#include <core/SkFont.h>
#include <core/SkPaint.h>
#include <core/SkRect.h>
class SkCanvas {
public:
    explicit SkCanvas(SkBitmap const&) { }
    void save() { }
    void restore() { }
    void clipRect(SkRect const&) { }
    void drawSimpleText(void const*, size_t, SkTextEncoding, float, float, SkFont const&, SkPaint const&) { }
};
EOF
cat > "$BOUCHONS/ports/SkFontMgr_fontconfig.h" <<'EOF'
#pragma once
#include <core/SkFontMgr.h>
inline sk_sp<SkFontMgr> SkFontMgr_New_FontConfig(void*, sk_sp<SkFontScanner>) { return {}; }
EOF
cat > "$BOUCHONS/ports/SkFontScanner_FreeType.h" <<'EOF'
#pragma once
#include <core/SkFontMgr.h>
inline sk_sp<SkFontScanner> SkFontScanner_Make_FreeType() { return {}; }
EOF

for export in LibCore LibWeb LibGfx LibIPC LibJS LibGC LibURL LibUnicode LibHTTP LibCrypto LibTLS LibWebView LibMedia LibRegex LibTextCodec LibThreading LibXML LibCompress LibWasm LibFileSystem LibSyntax LibDiff LibLine LibMain LibRIFF LibDevTools LibAudio LibRequests LibWebSocket; do
    bouchon_export "$export/Export.h"
done

UNITE="$BOUCHONS/unite.cpp"
{
    echo '#define BOUCHAUD_PORT 1'
    echo '#include <WebContent/BouchaudChrome.h>'
    # Une unite de traduction qui n'instancie rien ne verifie que les
    # declarations. Le chrome est entierement `inline` : ce sont ses corps qu'on
    # veut analyser, et le compilateur les analyse des qu'il les voit.
    echo 'int main() { return 0; }'
} > "$UNITE"

# `-Werror` n'est PAS passe au compilateur, et ce n'est pas un relachement.
#
# AK declenche a lui seul plusieurs avertissements que Ladybird desactive dans
# sa propre construction : un `operator""sv` sans souligne, un `offsetof` sur un
# type non standard-layout, un `move()` non qualifie. Les recopier ici en
# `-Wno-` reviendrait a tenir une seconde liste, qui divergerait de la premiere
# et finirait par masquer un avertissement du CHROME.
#
# La regle est donc portee ici : toute ERREUR compte, et tout AVERTISSEMENT
# dont le fichier est un des notres compte aussi. Ce qu'AK se dit a lui-meme ne
# nous regarde pas.
# La seule exception : `move()` et `forward()` sans qualification. Ce n'est pas
# un defaut a masquer, c'est la CONVENTION du projet -- ce sont `AK::move` et
# `AK::forward`, et tout le code de Ladybird les appelle ainsi. Ladybird
# desactive ce meme avertissement dans sa construction ; ne pas le desactiver
# ici ferait crier cette verification sur chaque `append(move(x))`.
JOURNAL="$BOUCHONS/journal.txt"
set +e
#
# `-Wno-invalid-constexpr` ferme un ecart de VERSION, pas un defaut : clang 18
# tient pour une erreur un constructeur `constexpr` d'`AK::Utf16String` que
# clang 20 -- celui avec lequel Ladybird se construit -- accepte. Le code
# incrimine est celui d'AK, et cette verification n'a pas a arbitrer.
"$CLANG" -std=c++2b -fsyntax-only -Wall -Wextra \
    -Wno-unqualified-std-cast-call -Wno-invalid-constexpr \
    "$UNITE" \
    -I "$BOUCHONS" -I "$SRC" -I "$SRC/Libraries" -I "$SRC/Services" \
    > "$JOURNAL" 2>&1
CODE=$?
set -e

NOTRES=$(grep -E "/Bouchaud[A-Za-z]*\.h:[0-9]+:[0-9]+: warning" "$JOURNAL" || true)
if [ "$CODE" -ne 0 ] || [ -n "$NOTRES" ]; then
    cat "$JOURNAL" >&2
    echo >&2
    if [ -n "$NOTRES" ]; then
        echo "syntaxe chrome : avertissement dans un fichier du chrome -- la" >&2
        echo "                 construction Ladybird les promeut en erreurs." >&2
    else
        echo "syntaxe chrome : le chrome ne compile pas -- voir ci-dessus" >&2
    fi
    exit 1
fi
printf '\033[32m%s\033[0m\n' "syntaxe chrome : $CHROME analyse sans erreur ni avertissement"
