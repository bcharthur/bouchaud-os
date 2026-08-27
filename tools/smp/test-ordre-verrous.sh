#!/bin/bash
# Ordre de prise des verrous du cache de pages propres.
#
# Un seul ordre est permis : CACHE -> Entry::state. Une inversion bloque deux
# CPU pour toujours, et aucune mesure ne la trouve : il faut que
# l'entrelacement se produise.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-ordre-verrous

echo "== ordre des verrous du cache de pages (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/smp/test_ordre_verrous.rs
"$SORTIE"
rm -f "$SORTIE"
