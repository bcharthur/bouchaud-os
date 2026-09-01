#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

OUT=${OUT:-../../target/security-runtime}
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
  security-ring3-probe.c \
  -o "$OUT/security-ring3-probe.o"

"$LD" \
  -static -n -z noexecstack --no-warn-rwx-segments \
  -Ttext-segment="$BASE" \
  -e _start \
  "$OUT/security-ring3-probe.o" \
  -o "$OUT/security-ring3-probe"

rm -f "$OUT/security-ring3-probe.o"
file "$OUT/security-ring3-probe"
echo "SECURITY_RING3_BUILD_OK"
