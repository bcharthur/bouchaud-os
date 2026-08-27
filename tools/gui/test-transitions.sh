#!/bin/bash
# Oracle de transition d'etat du compositeur.
#
#   rendu(A) + degats annonces par A -> B, appliques a B == rendu(B)
#
# Bit pour bit, sur TOUT le framebuffer simule. Un pixel qui differe hors degat
# est un etat qui change sans que personne ne l'annonce -- exactement le defaut
# de l'horloge sur la mauvaise barre, du survol qui oublie l'ancienne ligne et
# du focus qui oublie la fenetre qui le perd.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-transitions

echo "== oracle de transition d'etat (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/gui/test_transitions.rs
"$SORTIE"
rm -f "$SORTIE"
