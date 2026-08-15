#!/bin/bash
# Point d'entree du port Ladybird natif de Bouchaud OS jusqu'a M6.
#
#   tools/ladybird/native-m6.sh
#   tools/ladybird/native-m6.sh --cible
#
# Il part uniquement du depot Bouchaud + reseau :
#   1. recupere Ladybird au SHA epingle
#   2. reconstruit M1..M5 avec les scripts deja valides
#   3. construit les dependances graphiques upstream
#   4. construit libgfx_rust, LibCompress puis LibGfx
#   5. produit libgfx-probe

set -eu
cd "$(dirname "$0")/../.."

CIBLE=${1:-}
case "$CIBLE" in
    ""|--cible) ;;
    -h|--help)
        sed -n '2,12p' "$0"
        exit 0
        ;;
    *)
        echo "usage: $0 [--cible]" >&2
        exit 2
        ;;
esac

tools/ladybird/fetch.sh
tools/ladybird/build-tout.sh $CIBLE
tools/ladybird/build-vcpkg-gfx.sh
tools/ladybird/build-rust-gfx.sh $CIBLE
tools/ladybird/build-libcompress.sh $CIBLE
tools/ladybird/build-libgfx.sh $CIBLE

suffixe=hote
[ "$CIBLE" = "--cible" ] && suffixe=bouchaud

echo
printf '\033[32mLadybird natif M6 pret (%s)\033[0m\n' "$suffixe"
echo "  third_party/build-libgfx-$suffixe/libGfx.a"
echo "  third_party/build-libgfx-$suffixe/libgfx-probe"
