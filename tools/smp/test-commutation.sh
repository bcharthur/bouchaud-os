#!/bin/bash
# Tests d'unite de l'ORDRE de publication d'une tache sortante.
#
# `switch_to` et `preempt_from_irq` rendaient la tache sortante eligible pour
# les autres CPU AVANT que `switch_context` n'ait sauvegarde son sommet de
# pile, et le gros verrou est deja rendu entre les deux. Un autre CPU pouvait
# donc reprendre la tache sur le sommet de la commutation PRECEDENTE, donc
# faire tourner deux CPU sur une meme pile noyau.
#
# L'entrelacement est joue A LA MAIN : ces tests ne clignotent pas.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-commutation

echo "== tests d'unite de l'ordre de commutation (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/smp/test_commutation.rs
"$SORTIE"
rm -f "$SORTIE"
