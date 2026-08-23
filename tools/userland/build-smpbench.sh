#!/bin/sh
set -eu
CC=${CC:-musl-gcc}
OUT=${OUT:-out-smpbench}
mkdir -p "$OUT"
"$CC" -O2 -static -pthread smpbench.c -o "$OUT/smpbench"
echo "built $OUT/smpbench"
