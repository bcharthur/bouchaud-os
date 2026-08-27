#!/bin/bash
# Tests d'unite de la re-entree par interruption sur la file d'execution.
#
# Le panic runtime : « SpinLock recursive acquisition on CPU 0 » dans
# CpuLocal::enqueue, atteint depuis l'IRQ 8042 via le reveil du compositeur,
# pendant qu'une tache du meme CPU tenait deja cette file.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-runqueue-irq

echo "== tests d'unite de la re-entree IRQ sur la file d'execution (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/smp/test_runqueue_irq.rs
"$SORTIE"
rm -f "$SORTIE"
