#!/bin/bash
# Tests d'unite de la jauge de chargement du navigateur.
#
# La question a laquelle repond la jauge -- « au bout de combien de temps la
# page est-elle affichee ? » -- ne se verifie pas dans un journal : il faudrait
# chronometrer l'ecran a la main. Elle se demontre ici.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-jauge

echo "== tests d'unite de la jauge de chargement (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/gui/test_jauge.rs
"$SORTIE"
rm -f "$SORTIE"
