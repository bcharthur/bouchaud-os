#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

OUT=${OUT:-../../target/native-ipc-runtime}
CC=${CC:-gcc}
LD=${LD:-ld}
BASE=0x400000400000

mkdir -p "$OUT"

"$CC" \
  -c -O2 -Wall -Wextra \
  -fno-stack-protector \
  -fno-asynchronous-unwind-tables \
  -fno-builtin \
  -mcmodel=large \
  -fno-pie \
  -mno-red-zone \
  native-ipc-ring3-probe.c \
  -o "$OUT/native-ipc-ring3-probe.o"

"$LD" \
  -static -n -z noexecstack --no-warn-rwx-segments \
  -Ttext-segment="$BASE" \
  -e _start \
  "$OUT/native-ipc-ring3-probe.o" \
  -o "$OUT/native-ipc-ring3-probe"

rm -f "$OUT/native-ipc-ring3-probe.o"

file "$OUT/native-ipc-ring3-probe"
readelf -h "$OUT/native-ipc-ring3-probe" | grep -E 'Class:|Machine:|Type:|Entry point'
echo "NATIVE_IPC_RING3_PROBE_BUILD_OK"
