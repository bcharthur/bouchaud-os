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
export BO_HELLO_EXE=${TMPDIR:-/tmp}/bo-hello.exe

# La fixture est REGENEREE a chaque execution. Un binaire commite serait un
# fichier que personne ne relit ; regenere, il reste le produit d'un source
# lisible, et le generateur (Python) comme le parseur (Rust) sont ecrits
# separement a partir de la specification. Qu'ils s'accordent est donc une
# verification, pas une tautologie.
echo "== fixture hello.exe =="
python3 tools/exec/fabrique-hello-exe.py "$BO_HELLO_EXE"

echo
echo "== tests d'unite des formats d'executable (hote) =="
rustc --edition 2021 --test -o "$SORTIE" tools/exec/test_format.rs
"$SORTIE"
rm -f "$SORTIE" "$BO_HELLO_EXE"
