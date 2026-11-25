#!/usr/bin/env bash
# BOUCHAUD_DEMONTAGE_PAR_TABLES_V1 -- duree de munmap / brk (reduction) sur une
# plage grande et peu peuplee, dans le vrai noyau.
#
#   tools/ci/run_munmap_probe.sh BOOTIMAGE [TAILLE_MIO] [PAGES] [REPETITIONS] [SMP]
#
# Imprime les lignes MUNMAP_PROBE* emises par l'invite ; code 0 si la sonde a
# conclu (MUNMAP_PROBE_OK) sans faute noyau.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_munmap_probe.sh BOOTIMAGE [TAILLE_MIO] [PAGES] [REPETITIONS] [SMP]}
TAILLE=${2:-1024}
PAGES=${3:-8}
REPS=${4:-5}
SMP=${5:-1}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
musl-gcc -O2 -static-pie -fPIE -Wl,-z,noexecstack tools/userland/munmap-probe.c -o "$T/munmap-probe"
if readelf -l "$T/munmap-probe" | grep -q 'Requesting program interpreter'; then
  echo 'munmap-probe doit etre statique' >&2
  exit 1
fi
mkdir -p "$T/scenario/bin"
cp "$T/munmap-probe" "$T/scenario/bin/munmap-probe"
cat > "$T/scenario/autorun" <<AUTORUN
echo MUNMAP_PROBE_BEGIN && /bin/munmap-probe $TAILLE $PAGES $REPS && echo MUNMAP_PROBE_FIN_OK
AUTORUN
(cd tools/userland && IMAGE="$T/probe.img" ./mkdisk.sh "$T/scenario")
set +e
timeout 300 qemu-system-x86_64 \
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
grep -aoE 'MUNMAP_PROBE[A-Z_]*( .*)?$' "$T/emis.log" || true
[ "$code" -ne 124 ] || { echo 'QEMU : delai depasse' >&2; exit 1; }
if grep -aqE '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|FAULT_FATAL|panicked at' "$T/serie.log"; then
  echo 'faute noyau pendant la sonde' >&2
  exit 1
fi
grep -aqF 'MUNMAP_PROBE_OK' "$T/emis.log"
