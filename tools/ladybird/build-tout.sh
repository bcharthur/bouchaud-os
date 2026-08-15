#!/bin/bash
# Construit tout le portage Ladybird, dans l'ordre, depuis un arbre neuf.
#
#   ./tools/ladybird/build-tout.sh          hote
#   ./tools/ladybird/build-tout.sh --cible  Bouchaud (static-pie)
#
# ## Pourquoi cet ordre, et pourquoi LibCore revient a la fin
#
# Le graphe de Ladybird n'est pas un arbre : LibCore reference LibUnicode, LibURL
# et LibTextCodec, qui sont construites avec LibJS, qui a besoin de LibCore. Au
# niveau des **archives**, la dependance est circulaire.
#
# Elle ne l'est pas au niveau des **objets** : compiler LibCore ne demande que
# des en-tetes, et compiler LibUnicode ne demande pas `libCore.a`. Seule
# l'edition de liens d'un executable a besoin de tout le monde a la fois.
#
# D'ou la marche : produire les archives dans l'ordre des dependances de
# compilation, puis revenir relier les temoins. `build-libcore.sh` sait qu'il
# peut etre appele trop tot et remet alors son temoin a plus tard, plutot que
# d'echouer sur une edition de liens qui ne peut pas encore aboutir.

set -eu
cd "$(dirname "$0")/../.."

CIBLE=${1:-}
etiquette=$([ "$CIBLE" = "--cible" ] && echo bouchaud || echo hote)

vert() { printf '\033[32m%s\033[0m\n' "$*"; }
gras() { printf '\033[1;36m\n=== %s ===\033[0m\n' "$*"; }

etape() {
    gras "$1 ($etiquette)"
    shift
    "$@"
}

# 1. Ce qui ne depend de rien.
etape "en-tetes generes"        ./tools/ladybird/build-entetes.sh $CIBLE
etape "dependances tierces"     ./tools/ladybird/build-deps.sh    $CIBLE
etape "ICU 77"                  ./tools/ladybird/build-icu.sh     $CIBLE
etape "caisses Rust"            ./tools/ladybird/build-rust.sh    $CIBLE

# Les en-tetes FFI n'existent qu'apres les caisses Rust : on repasse.
etape "en-tetes generes (FFI)"  ./tools/ladybird/build-entetes.sh $CIBLE

# 2. La pile C++, dans l'ordre des dependances de compilation.
etape "AK"                      ./tools/ladybird/build-ak.sh       $CIBLE
etape "LibSync + LibCore"       ./tools/ladybird/build-libcore.sh  $CIBLE
etape "LibUnicode"              ./tools/ladybird/build-libunicode.sh $CIBLE
etape "LibCrypto"               ./tools/ladybird/build-libcrypto.sh  $CIBLE
etape "LibGC"                   ./tools/ladybird/build-libgc.sh      $CIBLE
etape "LibJS + satellites"      ./tools/ladybird/build-libjs.sh      $CIBLE

# 3. Le temoin de LibCore, maintenant que LibUnicode et LibJS existent.
etape "LibCore (temoin)"        ./tools/ladybird/build-libcore.sh  $CIBLE

# 4. `js`, l'interpreteur d'upstream : l'executable reel du portage.
etape "js (upstream)"           ./tools/ladybird/build-js.sh       $CIBLE

echo
vert "portage Ladybird complet pour la cible « $etiquette »"
