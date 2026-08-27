#!/bin/bash
# Cout d'une liberation dans le cache de pages propres.
#
# `release` cherchait la cle par balayage puis COMPTAIT les entrees
# recuperables en prenant le verrou de chaque entree -- a chaque liberation.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-cache-pages

echo "== cout des liberations du cache de pages propres (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/smp/test_cache_pages.rs
"$SORTIE"
rm -f "$SORTIE"
