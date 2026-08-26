#!/bin/bash
# Tests d'unite du decodage clavier, compiles pour l'hote.
#
# Ce que la compilation du noyau ne peut pas dire : qu'une touche tapee produit
# un appui ET un relachement, que le relachement porte la meme touche, qu'une
# touche maintenue se declare repetition, et qu'un modificateur relache cesse
# d'agir. Aucun de ces defauts ne se voit au boot.
#
# Ne demande ni QEMU, ni Qt, ni le reseau.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-clavier

echo "== tests d'unite du decodage clavier (hote) =="
rustc --edition 2021 --test -o "$SORTIE" tools/gui/test_clavier.rs
"$SORTIE"
rm -f "$SORTIE"
