#!/bin/bash
# Decoupe du flux serie en lots de FIFO : memes octets, meme ordre.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-lots

echo "== lots du port serie (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/serie/test_lots.rs
"$SORTIE"
rm -f "$SORTIE"
