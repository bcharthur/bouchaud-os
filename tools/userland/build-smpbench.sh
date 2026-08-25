#!/bin/sh
set -eu
CC=${CC:-musl-gcc}
OUT=${OUT:-out-smpbench}
mkdir -p "$OUT"
"$CC" -O2 -static -pthread smpbench.c -o "$OUT/smpbench"
"$CC" -O2 -static -pthread smpmix.c -o "$OUT/smpmix"
echo "built $OUT/smpbench and $OUT/smpmix"
