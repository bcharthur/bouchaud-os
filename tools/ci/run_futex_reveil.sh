#!/usr/bin/env bash
# BOUCHAUD_FUTEX_REVEIL_COMPTE_V1 -- reproducteur deterministe de #376 dans le
# vrai noyau : un attendant futex temporise, COMPTE par FUTEX_WAKE, ne doit
# jamais rendre ETIMEDOUT (tools/userland/futex-reveil-echeance.c).
#
#   tools/ci/run_futex_reveil.sh BOOTIMAGE [TOURS] [SMP]
#
# Code 0 si FUTEX_REVEIL_OK sans faute noyau.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_futex_reveil.sh BOOTIMAGE [TOURS] [SMP]}
TOURS=${2:-40}
SMP=${3:-1}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
gcc -O2 -static-pie -fPIE -pthread -Wl,-z,noexecstack tools/userland/futex-reveil-echeance.c -o "$T/futex-reveil-echeance"
if readelf -l "$T/futex-reveil-echeance" | grep 'Requesting program interpreter' >/dev/null; then
  echo 'futex-reveil-echeance doit etre statique' >&2
  exit 1
fi
mkdir -p "$T/scenario/bin"
cp "$T/futex-reveil-echeance" "$T/scenario/bin/futex-reveil-echeance"
cat > "$T/scenario/autorun" <<AUTORUN
echo FUTEX_REVEIL_BEGIN ; /bin/futex-reveil-echeance $TOURS ; echo FUTEX_REVEIL_FIN
AUTORUN
(cd tools/userland && IMAGE="$T/probe.img" ./mkdisk.sh "$T/scenario")
set +e
timeout 600 qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$T/probe.img" \
  -m 2048 -smp "$SMP" -accel tcg -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -serial file:"$T/serie.log"
code=$?
set -e
sed 's/\x1b\[[0-9;]*[A-Za-z]//g' "$T/serie.log" \
  | grep -avE '^[[:space:]]*(\[[0-9:]{8}\]\[[^]]*\]\[FPS:[^]]*\][[:space:]]*)?\+ ' > "$T/emis.log" || true
grep -aoE 'FUTEX_REVEIL[A-Z_]*( .*)?$' "$T/emis.log" || true
[ "$code" -ne 124 ] || { echo 'QEMU : delai depasse' >&2; exit 1; }
if grep -aqE '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$T/serie.log"; then
  echo 'faute noyau pendant la sonde' >&2
  exit 1
fi
grep -aqF 'FUTEX_REVEIL_OK' "$T/emis.log"
