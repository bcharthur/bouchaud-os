#!/usr/bin/env bash
# BOUCHAUD_ESPACE_RENDU_A_LA_MORT_V1 + BOUCHAUD_OOM_CONTROLE_V1 -- la memoire
# d'un mort revient a sa mort ; a court de memoire, le gouffre meurt, pas le
# fautif (tools/userland/memoire-mort-probe.c).
#
#   tools/ci/run_memoire_mort.sh BOOTIMAGE [MODES] [SMP] [MEMOIRE_MIO]
#
# MODES : "zombie oom" par defaut. Code 0 si chaque mode imprime
# MEMOIRE_MORT_OK, sans faute noyau. BO_QEMU_KVM=1 impose KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_memoire_mort.sh BOOTIMAGE [MODES] [SMP] [MEMOIRE_MIO]}
MODES=${2:-zombie oom}
SMP=${3:-2}
MEMOIRE=${4:-1024}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
gcc -O2 -pthread -static-pie -fPIE -Wl,-z,noexecstack tools/userland/memoire-mort-probe.c -o "$T/memoire-mort-probe"
mkdir -p "$T/scenario/bin"
cp "$T/memoire-mort-probe" "$T/scenario/bin/"
cp "$BOOT" "$T/boot.bin"
{
  for m in $MODES; do
    echo "echo MEMOIRE_MORT_ETAPE $m ; /bin/memoire-mort-probe $m"
  done
  echo "echo MEMOIRE_MORT_FIN"
} > "$T/scenario/autorun"
(cd tools/userland && IMAGE="$T/probe.img" ./mkdisk.sh "$T/scenario" >/dev/null)
accel=(-accel tcg -cpu max)
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "/dev/kvm inaccessible" >&2; exit 1; }
  accel=(-accel kvm -cpu host)
fi
set +e
timeout 900 qemu-system-x86_64 \
  -drive format=raw,file="$T/boot.bin" \
  -drive format=raw,file="$T/probe.img" \
  -m "$MEMOIRE" -smp "$SMP" "${accel[@]}" -display none -no-reboot \
  -netdev user,id=net0 -device e1000,netdev=net0 \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -serial file:"$T/serie.log"
code=$?
set -e
sed 's/\x1b\[[0-9;]*[A-Za-z]//g' "$T/serie.log" > "$T/emis.log"
if [ -n "${MEMOIRE_JOURNAL:-}" ]; then cp "$T/emis.log" "$MEMOIRE_JOURNAL"; fi
grep -aoE 'MEMOIRE_MORT_[A-Z]+( .*)?$|OOM_(VICTIME|ABANDON) .*|PROCESS_FAULT .*' "$T/emis.log" || true
[ "$code" -ne 124 ] || { echo 'QEMU : delai depasse' >&2; exit 1; }
if grep -aqE '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$T/serie.log"; then
  echo 'faute noyau pendant la sonde' >&2
  exit 1
fi
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  grep -aE 'BOUCHAUD_CPU_ISA hyperviseur=KVMKVMKVM([[:space:]]|$)' "$T/emis.log"
fi
for m in $MODES; do
  grep -aqF "MEMOIRE_MORT_OK mode=$m" "$T/emis.log" || { echo "mode $m : pas de MEMOIRE_MORT_OK" >&2; exit 1; }
done
