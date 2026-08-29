#!/bin/bash
# Decoupe du texte : la rasterisation culled ecrit-elle les memes pixels ?
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-texte

echo "== decoupe du texte (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/gui/test_texte.rs
"$SORTIE"
rm -f "$SORTIE"
