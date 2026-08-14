#!/bin/bash
# Verifications du protocole GUI userland.
#
# Deux barrieres, qui n'attrapent pas la meme chose :
#
#   1. les tests d'unite de `src/gui/protocole.rs`, compiles pour l'hote —
#      rognage des degats, conversion des coordonnees, cadrage du flux ;
#   2. la coherence entre les deux implementations du format, celle du noyau en
#      Rust et celle de l'hote Qt en C++ (`tools/verifie-protocole-gui.py`).
#
# Aucune des deux ne demande QEMU, Qt, ni le reseau : elles tournent en une
# seconde sur n'importe quelle machine, ce qui est la condition pour qu'on les
# lance vraiment.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-protocole

echo "== tests d'unite du protocole (hote) =="
rustc --edition 2021 --test -o "$SORTIE" tools/gui/test_protocole.rs
"$SORTIE"
rm -f "$SORTIE"

echo
echo "== coherence Rust / C++ =="
python3 tools/verifie-protocole-gui.py
