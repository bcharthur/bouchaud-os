#!/bin/bash
# Le protocole d'accuse de reception des evenements d'entree (A0).
#
# Un accuse rendu par WebContent pour UNE mise en file faite par l'hote : ni
# plus, ni moins. Un accuse de trop, et `m_pending_input_events.dequeue()`
# tombe sur une file vide -- c'est le `VERIFICATION FAILED: !is_empty() at
# AK/Queue.h:50` qui tuait le navigateur des le premier clic. Un accuse de
# moins, et la file de l'hote grossit sans fin, son entree se bloque.
#
# La sonde rejoue les deux cotes du protocole et verifie les deux sens. Elle
# rejoue aussi chaque scenario avec le portage d'AVANT A0, qui doit echouer :
# une sonde qui passe des deux cotes ne prouverait rien.
#
# Elle ne demande ni Ladybird, ni vcpkg, ni QEMU : quelques secondes sur
# n'importe quelle machine. Ce qu'elle ne peut pas faire, c'est compiler
# LibWeb ; le controle qui fait autorite reste `browser-upstream.sh` suivi d'un
# vrai clic dans QEMU.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-input-ownership-probe
CXX=${CXX:-c++}

echo "== protocole d'entree : accuse de reception (A0) =="
"$CXX" -std=c++17 -Wall -Wextra -Werror -o "$SORTIE" tools/ladybird/input-ownership-probe.cpp
"$SORTIE"
rm -f "$SORTIE"
