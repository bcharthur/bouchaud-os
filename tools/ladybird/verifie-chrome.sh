#!/bin/bash
# Compile `chrome/BouchaudChrome.h` seul, avec les avertissements de Ladybird.
#
# ## Pourquoi ce script existe
#
# Le chrome M11 n'est compile nulle part ailleurs qu'au milieu du build Ladybird
# complet : quinze minutes de ccache tiede, une heure et demie a froid. Une
# faute de frappe s'y paie donc au prix d'un aller-retour de CI.
#
# Ce que le premier aller-retour a coute est instructif : les erreurs n'etaient
# pas des fautes de syntaxe mais des **avertissements promus en erreurs** par
# upstream — `-Wcomment` sur une barre oblique inverse en fin de commentaire,
# `-Wexit-time-destructors` sur un `static` a destructeur,
# `-Wwritable-strings` sur `getenv` qui rend `char*`. Compiler « sans
# avertissements » ne prouvait rien : c'est le jeu d'avertissements de Ladybird
# qu'il faut reproduire, et il est copie ci-dessous depuis la ligne de commande
# reelle de la CI.
#
# ## Ce qu'il ne prouve pas
#
# Uniquement l'en-tete. Le code greffe dans `ConnectionFromClient.cpp` et
# `PageClient.cpp` par `prepare-m11-chrome.py` a besoin des en-tetes generes de
# LibWeb, donc du build complet. Ce script reduit la surface non verifiee, il ne
# l'annule pas.
#
# ## Usage
#
#   tools/ladybird/verifie-chrome.sh          # exige third_party/ladybird
#
set -euo pipefail
cd "$(dirname "$0")/../.."
RACINE=$(pwd)

LB="$RACINE/third_party/ladybird"
CHROME="$RACINE/tools/ladybird/chrome/BouchaudChrome.h"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf "\033[32m%s\033[0m\n" "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

if [ ! -d "$LB/AK" ]; then
    rouge "arbre Ladybird absent : lancer tools/ladybird/fetch.sh d'abord"
    exit 1
fi

CLANGXX=""
for candidat in clang++-20 clang++-19 clang++-18 clang++; do
    if command -v "$candidat" >/dev/null 2>&1; then
        CLANGXX=$(command -v "$candidat")
        break
    fi
done
if [ -z "$CLANGXX" ]; then
    rouge "aucun clang++ trouve"
    exit 1
fi

info "== chrome M11 contre $($CLANGXX --version | head -n1) =="

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# En-tetes que CMake genere et que nous n'avons pas ici. Les bouchonner suffit :
# ils ne definissent que des macros de visibilite, vides dans une compilation
# statique comme la notre.
mkdir -p "$TMP/stub/AK"
printf '#pragma once\n#define UTF8_DEBUG 0\n#define LIBWEB_CSS_DEBUG 0\n' > "$TMP/stub/AK/Debug.h"
printf '#pragma once\n#define AK_API\n' > "$TMP/stub/AK/Export.h"
for lib in Web Gfx Core IPC URL GC JS Unicode Regex Syntax TextCodec Crypto HTTP Media WebView DNS Requests Threading Sync FileSystem Compress; do
    mkdir -p "$TMP/stub/Lib$lib"
    majuscules=$(printf '%s' "$lib" | tr '[:lower:]' '[:upper:]')
    printf '#pragma once\n#define %s_API\n' "$majuscules" > "$TMP/stub/Lib$lib/Export.h"
done

mkdir -p "$TMP/WebContent"
cp "$CHROME" "$TMP/WebContent/BouchaudChrome.h"
cat > "$TMP/tu.cpp" <<'TU'
#define BOUCHAUD_PORT 1
#include <WebContent/BouchaudChrome.h>
TU

# Copie conforme des avertissements de `Meta/CMake/compile_options.cmake` tels
# que la CI les passe. `-Wno-error=invalid-constexpr` est celui qu'upstream pose
# deja lui-meme pour cette chaine.
"$CLANGXX" \
    -std=c++23 \
    -fsyntax-only \
    -DBOUCHAUD_PORT=1 \
    -I"$LB" \
    -I"$LB/Libraries" \
    -I"$LB/Services" \
    -I"$TMP" \
    -I"$TMP/stub" \
    -O3 -DNDEBUG \
    -march=x86-64 -mtune=generic \
    -fno-exceptions \
    -Werror \
    -Wall -Wextra \
    -Wcast-qual \
    -Wformat=2 \
    -Wimplicit-fallthrough \
    -Wmissing-declarations \
    -Wmissing-field-initializers \
    -Wmissing-prototypes \
    -Wsuggest-override \
    -Wexit-time-destructors \
    -Wpadded-bitfield \
    -Wno-error=invalid-constexpr \
    -Wno-expansion-to-defined \
    -Wno-invalid-offsetof \
    -Wno-shorten-64-to-32 \
    -Wno-unknown-warning-option \
    -Wno-unused-command-line-argument \
    -Wno-user-defined-literals \
    -Wno-implicit-const-int-float-conversion \
    -Wno-unqualified-std-cast-call \
    -Wno-c23-extensions \
    "$TMP/tu.cpp"

printf '\033[32m%s\033[0m\n' "chrome M11 : compile sans avertissement"
