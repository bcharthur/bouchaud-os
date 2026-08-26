#!/bin/bash
# Tests d'unite de l'identification des formats d'executable.
#
# Reconnaitre un format est une fonction des seuls octets du fichier : aucun
# espace d'adressage, aucun verrou, aucun descripteur. Cela s'exerce donc ici,
# sans QEMU.
#
# Ce que ces tests protegent : un `.exe` doit etre NOMME comme tel, un MZ sans
# signature PE ne doit pas etre pris pour un PE32+, un offset qui deborde ne
# doit pas faire lire hors du fichier, et un binaire du runtime Bouchaud ne doit
# pas etre confondu avec un binaire Windows.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-format-exec

echo "== tests d'unite des formats d'executable (hote) =="
rustc --edition 2021 --test -o "$SORTIE" tools/exec/test_format.rs
"$SORTIE"
rm -f "$SORTIE"
