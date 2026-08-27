#!/bin/bash
# Tests d'unite du culling de scene (Gate 1C).
#
# Quels calques dessiner pour un rectangle donne, et dans quel ordre. Une regle
# de culling fausse ne se voit pas dans un compteur : elle se voit a l'ecran,
# sous forme de trainee ou de disparition, parfois seulement dans une
# configuration de fenetres particuliere.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-scene

echo "== tests d'unite du culling de scene (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/gui/test_scene.rs
"$SORTIE"
rm -f "$SORTIE"
