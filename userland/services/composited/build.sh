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

"$LD" \
  -static -n -z noexecstack --no-warn-rwx-segments \
  -Ttext-segment="$BASE" \
  -e _start \
  "$OUT/composited-slice.o" \
  -o "$OUT/composited-slice"

rm -f "$OUT/composited-slice.o"

file "$OUT/composited-slice"
readelf -h "$OUT/composited-slice" | grep -E 'Class:|Machine:|Type:|Entry point'
echo "COMPOSITED_SLICE_BUILD_OK"
