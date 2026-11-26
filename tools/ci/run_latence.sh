#!/usr/bin/env bash
# BOUCHAUD_LATENCE_INTERACTIVE_V1 -- retard de reveil d'un processus
# interactif (1 ms) pendant que le systeme calcule, forke, demappe, ecrit sur
# la console, lance des processus (tools/userland/latence-probe.c).
#
#   tools/ci/run_latence.sh BOOTIMAGE [SMP] [SECONDES_PAR_PHASE] [PHASE] [NICE]
#
# Mesure seulement (pas de seuil) : sortie 0 si LATENCE_PROBE_FIN sans faute
# noyau. GARDE_JOURNAL=fichier garde le journal serie.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_latence.sh BOOTIMAGE [SMP] [SECONDES_PAR_PHASE]}
SMP=${2:-1}
SEC=${3:-3}
PHASE=${4:-toutes}
NICE=${5:--5}
T=$(mktemp -d)
trap '[ -n "${GARDE_JOURNAL:-}" ] && cp "$T/serie.log" "$GARDE_JOURNAL"; rm -rf "$T"' EXIT
gcc -O2 -static-pie -fPIE tools/userland/latence-probe.c -o "$T/latence-probe"
mkdir -p "$T/scenario/bin"
cp "$T/latence-probe" "$T/scenario/bin/"
cp "$BOOT" "$T/boot.bin"
echo "echo LATENCE_PROBE_BANC ; /bin/latence-probe $SEC $PHASE $NICE" > "$T/scenario/autorun"
(cd tools/userland && IMAGE="$T/probe.img" ./mkdisk.sh "$T/scenario" >/dev/null)
accel=(-accel tcg -cpu max)
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  test -r /dev/kvm && test -w /dev/kvm || { echo 'KVM indisponible' >&2; exit 1; }
  accel=(-accel kvm -cpu host)
fi
set +e
timeout ${BO_DELAI_S:-900} qemu-system-x86_64 \
  -drive format=raw,file="$T/boot.bin" \
  -drive format=raw,file="$T/probe.img" \
  -m 1024 -smp "$SMP" "${accel[@]}" -display none -no-reboot \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
  -serial file:"$T/serie.log"
code=$?
set -e
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  grep -aqE 'BOUCHAUD_CPU_ISA hyperviseur=KVMKVMKVM([[:space:]]|$)' "$T/serie.log"
fi
sed 's/\x1b\[[0-9;]*[A-Za-z]//g' "$T/serie.log" > "$T/emis.log"
grep -aoE 'LATENCE_PROBE phase=.*$|LATENCE_PROBE_FIN|PROCESS_FAULT .*' "$T/emis.log" || true
[ "$code" -ne 124 ] || { echo 'QEMU : delai depasse' >&2; exit 1; }
if grep -aqE '\*\*\* KERNEL PANIC \*\*\*|DOUBLE FAULT|panicked at' "$T/serie.log"; then
  echo 'faute noyau pendant la sonde' >&2
  exit 1
fi
if [ "$PHASE" != toutes ]; then
  grep -aqE "LATENCE_PROBE phase=${PHASE} n=[1-9][0-9]* p50_us=" "$T/emis.log"
fi
grep -aq 'LATENCE_PROBE_FIN' "$T/emis.log"
