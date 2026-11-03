#!/usr/bin/env bash
# Build ONE native Bouchaud program against bouchaud-sdk.
#
#   build-program.sh OUTPUT SOURCE.c [SOURCE2.c ...]
#
# Freestanding, static, W^X (one R+X segment, one RW segment), no libc. The
# result is checked here: an image the kernel would refuse fails the build,
# not a boot three minutes later.
set -euo pipefail
SDK="$(cd "$(dirname "$0")" && pwd)"
OUT=${1:?usage: build-program.sh OUTPUT SOURCE.c...}
shift
CC=${CC:-gcc}
LD=${LD:-ld}
BASE=${BO_TEXT_BASE:-0x400000400000}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
objs=()
for src in "$SDK/src/runtime.c" "$@"; do
  obj="$tmp/$(basename "$src" .c).$(echo "$src" | md5sum | cut -c1-8).o"
  "$CC" -c -O2 -Wall -Wextra -Werror \
    -std=gnu11 \
    -fno-stack-protector -fno-asynchronous-unwind-tables -fno-builtin \
    -fno-pie -mcmodel=large -mno-red-zone -ffreestanding -nostdinc \
    -I "$SDK/include" \
    "$src" -o "$obj"
  objs+=("$obj")
done

mkdir -p "$(dirname "$OUT")"
"$LD" -static -z noexecstack -z separate-code \
  -Ttext-segment="$BASE" -e _start \
  "${objs[@]}" -o "$OUT"

if readelf -lW "$OUT" | grep -qE '^  LOAD .* RWE '; then
  echo "BOUCHAUD_SDK_WX: segment inscriptible ET executable dans $OUT" >&2
  exit 1
fi
if readelf -sW "$OUT" | grep -qE ' UND [a-z_]+$'; then
  echo "BOUCHAUD_SDK_UNDEF: symbole non resolu dans $OUT" >&2
  readelf -sW "$OUT" | grep ' UND ' >&2
  exit 1
fi
echo "BOUCHAUD_SDK_BUILD_OK $(basename "$OUT") $(sha256sum "$OUT" | cut -c1-16)"
