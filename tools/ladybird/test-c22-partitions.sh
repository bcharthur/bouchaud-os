#!/usr/bin/env bash
# C22 -- banc hote des tas mimalloc partitionnes (BOUCHAUD_C22_PARTITIONS_COLLECTEES_V1).
#
#   tools/ladybird/test-c22-partitions.sh ARBRE_LADYBIRD SOURCES_MIMALLOC
#
# ARBRE_LADYBIRD : l'arbre AU SHA EPINGLE (third_party/UPSTREAM.md), non
# prepare ; SOURCES_MIMALLOC : mimalloc a la version de build-deps.sh.
# Construit AK/kmalloc.cpp d'origine et prepare, joue test_c22_partitions.cpp
# contre les deux, puis le correctif sous BOUCHAUD_LB_C22_TEMOIN_HISTORIQUE=1.
# Exige : origine ET temoin retiennent >= 1 Mio apres le crochet (le defaut
# est reproduit), le correctif <= 512 Kio, en phase 1 comme en phase 2.
set -euo pipefail
cd "$(dirname "$0")/../.."
LB=${1:?usage: test-c22-partitions.sh ARBRE_LADYBIRD SOURCES_MIMALLOC}
MI=${2:?usage: test-c22-partitions.sh ARBRE_LADYBIRD SOURCES_MIMALLOC}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/arbre/AK" "$T/arbre/Libraries/LibWeb/DOM"
cp "$LB/Libraries/LibWeb/DOM/Document.cpp" "$T/arbre/Libraries/LibWeb/DOM/Document.cpp"
cp "$LB/AK/kmalloc.cpp" "$T/kmalloc-origine.cpp"
cp "$LB/AK/kmalloc.cpp" "$T/arbre/AK/kmalloc.cpp"
python3 tools/ladybird/prepare-c22-partitions.py "$T/arbre"
# Le correctif doit etre relie au demontage du Document, PAS uniquement au GC.
grep -qF "    ak_kmalloc_collect();" "$T/arbre/Libraries/LibWeb/DOM/Document.cpp"
grep -qF "BOUCHAUD_C22_DOCUMENT_COLLECTE_V2" "$T/arbre/Libraries/LibWeb/DOM/Document.cpp"
cp "$T/arbre/AK/kmalloc.cpp" "$T/kmalloc-correctif.cpp"
printf '#include <cstdio>\n#include <cstdlib>\nextern "C" __attribute__((noreturn)) void ak_verification_failed(char const* m) { fprintf(stderr, "VERIFY %%s\\n", m); abort(); }\n' > "$T/stub.cpp"
cc -O2 -DNDEBUG -c "$MI/src/static.c" -I"$MI/include" -o "$T/mimalloc.o"
for v in origine correctif; do
  g++ -std=c++23 -O2 -I"$LB" -I"$MI/include" tools/ladybird/c22_partitions_fixture.cpp "$T/kmalloc-$v.cpp" "$T/stub.cpp" "$T/mimalloc.o" -lpthread -o "$T/test-$v"
done
echecs=0
juge() {
  local nom=$1 min=$2 max=$3; shift 3
  local sortie; sortie=$("$@")
  echo "$sortie" | sed "s/^/  $nom : /"
  for r in $(echo "$sortie" | grep -o 'retenu_kio=-\?[0-9]*' | cut -d= -f2); do
    if [ "$r" -lt "$min" ] || [ "$r" -gt "$max" ]; then echo "  ECHEC $nom retenu_kio=$r hors [$min,$max]"; echecs=$((echecs + 1)); fi
  done
}
juge origine 1024 1000000 "$T/test-origine"
juge temoin 1024 1000000 env BOUCHAUD_LB_C22_TEMOIN_HISTORIQUE=1 "$T/test-correctif"
juge correctif -1000000 512 "$T/test-correctif"
if [ "$echecs" -eq 0 ]; then echo "C22_PARTITIONS_HOTE_OK"; else echo "C22_PARTITIONS_HOTE_ECHEC n=$echecs"; exit 1; fi
