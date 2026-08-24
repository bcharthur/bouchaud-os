#!/bin/sh
set -eu
CC="${CC:-x86_64-linux-musl-gcc}"
"$CC" -O2 -static -pthread tools/userland/mmstress.c -o mmstress
echo "built: mmstress"
