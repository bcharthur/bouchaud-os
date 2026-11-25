#!/usr/bin/env bash
# BOUCHAUD_SHM_PROJECTION_V1 -- projection sans copie des SharedRegion
# natives (tools/userland/shm-map-probe.c, ABI 1.7).
#
#   tools/ci/run_shm_map.sh BOOTIMAGE [SMP]
#
# Code 0 si SHM_MAP_OK sans faute noyau. Banc court (TCG).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_shm_map.sh BOOTIMAGE [SMP]}
SMP=${2:-2}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
gcc -O2 -static-pie -fPIE -Wl,-z,noexecstack -Iuserland/libs/bouchaud-sdk/include \
  tools/userland/shm-map-probe.c -o "$T/shm-map-probe"
mkdir -p "$T/scenario/bin"
cp "$T/shm-map-probe" "$T/scenario/bin/"
cp "$BOOT" "$T/boot.bin"
echo "echo SHM_MAP_BANC ; /bin/shm-map-probe ; echo SHM_MAP_FIN" > "$T/scenario/autorun"
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
grep -aoE 'SHM_MAP_[A-Z_]+( .*)?$|PROCESS_FAULT .*' "$T/emis.log" | grep -av 'SHM_MAP_BANC ;' || true
[ "$code" -ne 124 ] || { echo 'QEMU : delai depasse' >&2; exit 1; }
if grep -aqE '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$T/serie.log"; then
  echo 'faute noyau pendant la sonde' >&2
  grep -aE -A6 'KERNEL PANIC|panicked at' "$T/serie.log" | awk 'NR<=20' >&2
  exit 1
fi
grep -aqE '[^A-Z_]SHM_MAP_OK|^SHM_MAP_OK' "$T/emis.log"
