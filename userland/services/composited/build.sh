#!/usr/bin/env bash
#
# Construit le tranchant vertical ring 3 de `composited`.
#
# Freestanding : aucune libc n'est liee. Le meme motif que
# `tools/userland/build-native-ipc-ring3-probe.sh`, pour la meme raison -- une
# sonde qui depend d'une libc ne prouve pas ce que l'ABI native sait faire, elle
# prouve ce que la libc sait contourner.
set -euo pipefail
cd "$(dirname "$0")"

OUT=${OUT:-../../../target/composited}
CC=${CC:-gcc}
LD=${LD:-ld}
BASE=0x400000400000

mkdir -p "$OUT"

"$CC" \
  -c -O2 -Wall -Wextra -Werror \
  -fno-stack-protector \
  -fno-asynchronous-unwind-tables \
  -fno-builtin \
  -mcmodel=large \
  -fno-pie \
  -mno-red-zone \
  -ffreestanding \
  composited-slice.c \
  -o "$OUT/composited-slice.o"

# W^X : DEUX SEGMENTS, PAS UN.
#
# L'edition de liens portait `-n` (nmagic), qui refuse d'aligner les sections
# sur des pages, et `--no-warn-rwx-segments`, qui faisait taire l'avertissement
# disant precisement ce qui n'allait pas. Le resultat etait UN seul segment
# LOAD portant `.text`, `.rodata` ET `.bss`, en RWE :
#
#     LOAD 0x000120 0x0000400000400120 ... RWE
#
# Le noyau le refusait, et il avait raison :
#
#     /bin/composited-slice : segment inscriptible et executable refuse (W^X)
#
# Le compositeur ring 3 ne s'executait donc JAMAIS. Il se construisait, la
# campagne validait sa construction, et le marqueur d'execution manquait --
# une preuve de compilation prise pour une preuve d'execution.
#
# Sans `-n`, et avec `-z separate-code`, l'editeur de liens produit ce qu'il
# faut : un segment R+X pour le code, un segment R/W pour les donnees. Le
# programme cesse d'etre refusable sans qu'on ait touche a la politique.
"$LD" \
  -static -z noexecstack -z separate-code \
  -Ttext-segment="$BASE" \
  -e _start \
  "$OUT/composited-slice.o" \
  -o "$OUT/composited-slice"

rm -f "$OUT/composited-slice.o"

# LA CONSTRUCTION VERIFIE CE QU'ELLE A PRODUIT.
#
# Un binaire refuse au chargement doit echouer ICI, pas trois minutes plus tard
# dans un journal serie, sous la forme d'un marqueur manquant qui n'explique
# rien.
if readelf -lW "$OUT/composited-slice" | grep -qE '^  LOAD .* RWE '; then
    echo "COMPOSITED_SLICE_WX : segment inscriptible ET executable produit ;" >&2
    echo "  le noyau le refusera au chargement (W^X)." >&2
    readelf -lW "$OUT/composited-slice" | grep -E '^  LOAD ' >&2
    exit 1
fi

file "$OUT/composited-slice"
readelf -h "$OUT/composited-slice" | grep -E 'Class:|Machine:|Type:|Entry point'
readelf -lW "$OUT/composited-slice" | grep -E '^  LOAD '
echo "COMPOSITED_SLICE_BUILD_OK"
