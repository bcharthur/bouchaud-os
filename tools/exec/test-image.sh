#!/bin/bash
# Tests d'unite de la PREPARATION d'une image executable.
#
# `pe::prepare` transforme des octets en description projetable : base, point
# d'entree absolu, segments et droits. C'est une fonction des seuls octets
# d'entree, donc exercable sans QEMU.
#
# Ces tests ne prouvent PAS qu'un .exe s'execute : cela demande un espace
# d'adressage et une machine. Ils prouvent que la description est correcte.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-image

echo "== tests d'unite de la preparation d'image (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/exec/test_image.rs
"$SORTIE"
rm -f "$SORTIE"
