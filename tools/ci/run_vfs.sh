#!/usr/bin/env bash
# BOUCHAUD_VFS_MONTAGES_V1 -- montages devfs/procfs, fichiers vivants
# (tools/userland/vfs-probe.c).
#
#   tools/ci/run_vfs.sh BOOTIMAGE [SMP]
#
# Code 0 si VFS_PROBE_OK sans faute noyau. Banc court (TCG).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_vfs.sh BOOTIMAGE [SMP]}
SMP=${2:-2}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
gcc -O2 -static-pie -fPIE -Wl,-z,noexecstack \
  tools/userland/vfs-probe.c -o "$T/vfs-probe"
mkdir -p "$T/scenario/bin"
cp "$T/vfs-probe" "$T/scenario/bin/"
cp "$BOOT" "$T/boot.bin"
echo "echo VFS_PROBE_BANC ; /bin/vfs-probe ; echo VFS_PROBE_FIN" > "$T/scenario/autorun"
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
grep -aoE 'VFS_PROBE_[A-Z_]+( .*)?$|PROCESS_FAULT .*' "$T/emis.log" | grep -av 'VFS_PROBE_BANC ;' || true
[ "$code" -ne 124 ] || { echo 'QEMU : delai depasse' >&2; exit 1; }
if grep -aqE '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$T/serie.log"; then
  echo 'faute noyau pendant la sonde' >&2
  grep -aE -A6 'KERNEL PANIC|panicked at' "$T/serie.log" | head -20 >&2
  exit 1
fi
grep -aqE '[^A-Z_]VFS_PROBE_OK|^VFS_PROBE_OK' "$T/emis.log"
