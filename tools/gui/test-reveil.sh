#!/bin/bash
# Tests d'unite de la politique event-driven du compositeur (Gate 1B).
#
# Deux choses : la politique elle-meme (`src/gui/politique.rs`, incluse telle
# quelle), et le protocole de reveil sans perte, modelise ici parce que le vrai
# vit sur une WaitQueue noyau.
#
# La fenetre « constate vide -> evenement -> tentative de sommeil » est rejouee
# a la main : ces tests ne clignotent pas.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-reveil

echo "== tests d'unite du compositeur event-driven (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/gui/test_reveil.rs
"$SORTIE"
rm -f "$SORTIE"
