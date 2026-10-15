#!/usr/bin/env bash
# P10+.3b -- A/B : DEUX COMMANDES PAR REGION CONTRE UNE TRAME, UNE SOUMISSION
# (BOUCHAUD_VIRTIO_GPU_TRAME_V1).
#
#   tools/ci/run_banc_trame.sh BOOTIMAGE [TOURS]
#
# Un demarrage `-vga none -device virtio-gpu-pci`, TOURS tours (6 par defaut)
# de `banc-trame` (100 trames de bureau, 8 regions) sous `gpu-trame off` et
# `gpu-trame on`, ordre AB aux tours impairs, BA aux pairs. Le demarrage finit
# sous `on` (le defaut depuis b6c99032). Criteres : tools/ci/analyse_banc_trame.py ; ECHEC sur
# un defaut de surete, CONVAINCANT / NON_CONVAINCANT / NON_CONCLUANT est une
# mesure.
#
# BO_QEMU_KVM=1 : sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_banc_trame.sh BOOTIMAGE [TOURS]}
TOURS=${2:-6}
ACCEL="-cpu max"
NOM=tcg
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
  NOM=kvm
fi
SORTIE=${BANC_SORTIE:-$PWD/banc-trame-$NOM}
rm -rf "$SORTIE"
mkdir -p "$SORTIE/scenario"
{
  for t in $(seq "$TOURS"); do
    if [ $((t % 2)) = 1 ]; then ordre="off on"; else ordre="on off"; fi
    for v in $ordre; do
      echo "echo AB_TRAME tour=$t trame=$v"
      echo "gpu-trame $v"
      echo "banc-trame"
    done
  done
  echo "gpu-trame on"
  echo "echo AB_TRAME_FIN"
} > "$SORTIE/scenario/autorun"
(cd tools/userland && IMAGE="$SORTIE/banc.img" ./mkdisk.sh "$SORTIE/scenario" >/dev/null)
LOG="$SORTIE/serie.log"
: > "$LOG"
qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/banc.img" \
  -m 2048 -smp 2 -display none -no-reboot -serial file:"$LOG" -vga none -device virtio-gpu-pci &
PID=$!
LIMITE=$((SECONDS + 180 + TOURS * 2 * 90))
while kill -0 "$PID" 2>/dev/null && (( SECONDS < LIMITE )); do
  # La sortie de `echo`, pas sa trace (`+ echo AB_TRAME_FIN`).
  if grep -aqE 'KERNEL PANIC|panicked at|\[BANC-TRAME\] ECHEC' "$LOG" \
     || sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'AB_TRAME_FIN' | grep -aqv '+ echo AB_TRAME_FIN'; then
    sleep 2; break
  fi
  sleep 2
done
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
python3 tools/ci/analyse_banc_trame.py "$LOG" "$TOURS" "$NOM"
