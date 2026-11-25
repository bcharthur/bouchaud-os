#!/usr/bin/env bash
# BOUCHAUD_ORPHELINS_RECOLTES_V1 -- sans init, le noyau recolte les orphelins
# (tools/userland/orphelin-probe.c).
#
#   tools/ci/run_orphelins.sh BOOTIMAGE [SMP]
#
# Code 0 si ORPHELIN_PROBE_OK sans faute noyau. Banc court (TCG).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_orphelins.sh BOOTIMAGE [SMP]}
SMP=${2:-2}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
gcc -O2 -static-pie -fPIE -Wl,-z,noexecstack tools/userland/orphelin-probe.c -o "$T/orphelin-probe"
mkdir -p "$T/scenario/bin"
cp "$T/orphelin-probe" "$T/scenario/bin/"
cp "$BOOT" "$T/boot.bin"
echo "echo ORPHELIN_BEGIN ; /bin/orphelin-probe ; echo ORPHELIN_FIN" > "$T/scenario/autorun"
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
grep -aoE 'ORPHELIN_(PROBE|RECOLTE)[A-Z_]*( .*)?$' "$T/emis.log" || true
[ "$code" -ne 124 ] || { echo 'QEMU : delai depasse' >&2; exit 1; }
if grep -aqE '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$T/serie.log"; then
  echo 'faute noyau pendant la sonde' >&2
  exit 1
fi
grep -aqF 'ORPHELIN_PROBE_OK' "$T/emis.log"
