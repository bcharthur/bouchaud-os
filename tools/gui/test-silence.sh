#!/bin/bash
# Tests d'unite du verdict « ce client annonce-t-il ses trames ? ».
#
# Le scenario qui a casse en production : Ladybird met plus de six secondes a
# demarrer sous TCG, depasse le delai de patience, est declare muet, puis se met
# a parler le protocole. Rien ne levait le verdict de silence, et le
# compositeur recomposait sa surface a l'aveugle pour toujours.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-silence

echo "== tests d'unite du verdict de protocole client (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/gui/test_silence.rs
"$SORTIE"
rm -f "$SORTIE"
