#!/bin/bash
# Cout d'un free_frame, et ce que son assertion doit continuer a detecter.
#
# La verification du double free parcourait toute la liste libre -- une liste
# chainee dans les frames liberees, donc une lecture memoire froide par pas.
# Liberer une plage de R pages coutait O(R x frames libres).
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-frames-libres

echo "== cout et correction de la liste de frames libres (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/smp/test_frames_libres.rs
"$SORTIE"
rm -f "$SORTIE"
