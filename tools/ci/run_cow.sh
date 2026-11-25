#!/usr/bin/env bash
# BOUCHAUD_COW_V1 -- fork en copie-a-l'ecriture dans le vrai noyau
# (tools/userland/cow-probe.c, glibc statique).
#
#   tools/ci/run_cow.sh BOOTIMAGE [N_RAFALE] [SMP] [MEMOIRE_MIO] [EPREUVE]
#
# Code 0 si COW_OK sans faute noyau. Banc court (TCG). Temoin : une image
# dont le fork recopie echoue a l'epreuve `memoire` (fork_recopie).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_cow.sh BOOTIMAGE [N_RAFALE] [SMP] [MEMOIRE_MIO]}
N=${2:-50}
SMP=${3:-2}
MEMOIRE=${4:-1024}
EPREUVE=${5:-}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
gcc -O2 -static-pie -fPIE -pthread -Wl,-z,noexecstack tools/userland/cow-probe.c -o "$T/cow-probe"
mkdir -p "$T/scenario/bin"
cp "$T/cow-probe" "$T/scenario/bin/"
cp "$BOOT" "$T/boot.bin"
echo "echo COW_BANC ; /bin/cow-probe $N $EPREUVE ; echo COW_FIN" > "$T/scenario/autorun"
(cd tools/userland && IMAGE="$T/probe.img" ./mkdisk.sh "$T/scenario" >/dev/null)
set +e
timeout 900 qemu-system-x86_64 \
  -drive format=raw,file="$T/boot.bin" \
  -drive format=raw,file="$T/probe.img" \
  -m "$MEMOIRE" -smp "$SMP" -accel tcg -cpu max -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -serial file:"$T/serie.log"
code=$?
set -e
sed 's/\x1b\[[0-9;]*[A-Za-z]//g' "$T/serie.log" > "$T/emis.log"
if [ -n "${COW_JOURNAL:-}" ]; then cp "$T/emis.log" "$COW_JOURNAL"; fi
grep -aoE 'COW_[A-Z]+( .*)?$|PROCESS_FAULT .*' "$T/emis.log" | grep -av 'COW_BANC ;' || true
grep -aoE 'PERF_FORK .*' "$T/emis.log" | tail -2 || true
[ "$code" -ne 124 ] || { echo 'QEMU : delai depasse' >&2; exit 1; }
if grep -aqE '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$T/serie.log"; then
  echo 'faute noyau pendant la sonde' >&2
  grep -aE -A6 'KERNEL PANIC|panicked at' "$T/serie.log" | head -20 >&2
  exit 1
fi
grep -aqE '^COW_OK|[^A-Z_]COW_OK' "$T/emis.log"
