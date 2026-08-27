#!/bin/bash
# Ce que madvise et poll ont le droit de faire sous le gros verrou.
#
# Sous le verrou, le cout d'une phase peut dependre de la taille de la DEMANDE ;
# jamais de la taille d'un ETAT GLOBAL. Et personne ne dort le verrou tenu.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-discipline-bkl

echo "== discipline du gros verrou (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/smp/test_discipline_bkl.rs
"$SORTIE"
rm -f "$SORTIE"
