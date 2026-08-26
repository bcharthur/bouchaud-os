#!/bin/bash
# Tests d'unite du CONTRAT DE PROFONDEUR du gros verrou.
#
# Toute primitive bloquante doit rendre la main en laissant le verrou a la
# profondeur exacte ou elle l'a trouve. Violer ce contrat ne produit aucune
# erreur sur le coup : la panique arrive plus tard, au Drop d'un garde
# quelconque, et accuse une fonction innocente.
#
# Ces tests modelisent les cinq formes de rupture (A..E) et verifient que la
# post-condition les nomme a la SOURCE.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-profondeur-bkl

echo "== tests d'unite du contrat de profondeur BKL (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/smp/test_profondeur_bkl.rs
"$SORTIE"
rm -f "$SORTIE"
