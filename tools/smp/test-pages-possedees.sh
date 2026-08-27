#!/bin/bash
# Cout des frames possedees par un espace d'adressage.
#
# `pages` etait un Vec balaye lineairement par prepare_unmap, finish_unmap et
# owns_frame -- soit O(R x P) pour madvise/munmap, et O(P^2) pour fork.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-pages-possedees

echo "== cout des frames possedees par un espace (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/smp/test_pages_possedees.rs
"$SORTIE"
rm -f "$SORTIE"
