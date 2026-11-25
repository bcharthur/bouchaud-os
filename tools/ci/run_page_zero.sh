#!/usr/bin/env bash
# BOUCHAUD_PAGE_ZERO_V1 -- une lecture anonyme ne coute pas de frame
# (tools/userland/page-zero-probe.c).
#
#   tools/ci/run_page_zero.sh BOOTIMAGE [SMP]
#
# Code 0 si PAGE_ZERO_OK sans faute noyau. Banc court (TCG).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_page_zero.sh BOOTIMAGE [SMP]}
SMP=${2:-2}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
gcc -O2 -static-pie -fPIE -Wl,-z,noexecstack \
  tools/userland/page-zero-probe.c -o "$T/page-zero-probe"
mkdir -p "$T/scenario/bin"
cp "$T/page-zero-probe" "$T/scenario/bin/"
cp "$BOOT" "$T/boot.bin"
echo "echo PAGE_ZERO_BANC ; /bin/page-zero-probe ; echo PAGE_ZERO_FIN" > "$T/scenario/autorun"
(cd tools/userland && IMAGE="$T/probe.img" ./mkdisk.sh "$T/scenario" >/dev/null)
set +e
timeout 600 qemu-system-x86_64 \
  -drive format=raw,file="$T/boot.bin" \
  -drive format=raw,file="$T/probe.img" \
  -m 1024 -smp "$SMP" -accel tcg -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -serial file:"$T/serie.log"
code=$?
set -e
sed 's/\x1b\[[0-9;]*[A-Za-z]//g' "$T/serie.log" > "$T/emis.log"
grep -aoE 'PAGE_ZERO_[A-Z_]+( .*)?$|PROCESS_FAULT .*' "$T/emis.log" | grep -av 'PAGE_ZERO_BANC ;' || true
[ "$code" -ne 124 ] || { echo 'QEMU : delai depasse' >&2; exit 1; }
if grep -aqE '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$T/serie.log"; then
  echo 'faute noyau pendant la sonde' >&2
  grep -aE -A6 'KERNEL PANIC|panicked at' "$T/serie.log" | awk 'NR<=20' >&2
  exit 1
fi
grep -aqE '[^A-Z_]PAGE_ZERO_OK|^PAGE_ZERO_OK' "$T/emis.log"
