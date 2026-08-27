#!/bin/bash
# Tests d'unite de la politique de degat du bureau.
#
# Ce qu'un evenement salit est de la geometrie pure. La regle a tenir --
# « cent clics et cent crans de molette ne produisent aucun degat plein
# ecran » -- se demontre ici, alors qu'un journal ne montrerait que la session
# qu'on a jouee.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-degats

echo "== tests d'unite de la politique de degat (hote) =="
rustc --edition 2021 --test -o "$SORTIE" tools/gui/test_degats.rs
"$SORTIE" --test-threads=1
rm -f "$SORTIE"
