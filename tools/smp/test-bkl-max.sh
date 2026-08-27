#!/bin/bash
# Tests d'unite de la publication du maximum de tenue du BKL.
#
# Deux proprietes qui se cassent en SILENCE : un maximum qui diminue parce que
# deux CPU l'ecrivent en meme temps, et une duree publiee avec la provenance
# d'une autre. La premiere fait chercher au mauvais endroit, la seconde accuse
# le mauvais appel systeme.
#
# Le modele reprend la formule du noyau et la soumet a de vrais fils
# concurrents -- ce qu'aucune relecture ne peut faire.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-bkl-max

echo "== tests d'unite du maximum de tenue BKL (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/smp/test_bkl_max.rs
"$SORTIE"
rm -f "$SORTIE"
