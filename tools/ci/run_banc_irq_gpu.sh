#!/usr/bin/env bash
# P10+.1 -- A/B : ATTENTE DES REPONSES VIRTIO-GPU PAR SCRUTATION OU PAR
# INTERRUPTION MSI-X (BOUCHAUD_VIRTIO_GPU_IRQ_V1).
#
#   tools/ci/run_banc_irq_gpu.sh BOOTIMAGE [TOURS]
#
# Un seul demarrage, ecran virtio-gpu seul. Chaque tour presente la meme
# suite de trames (`banc-present tache` : plein ecran, bande, tuile) depuis une
# tache noyau epinglee au coeur zero -- le contexte du bureau --, sous les deux
# politiques (`gpu-irq off` puis `on` aux tours impairs, l'inverse aux pairs).
# Le demarrage finit sous la politique par defaut (interruption, depuis
# l'A/B KVM CONVAINCANT de f23a1247).
#
# Criteres et verdicts : tools/ci/analyse_banc_irq_gpu.py (declares avant la
# mesure). Le banc ECHOUE sur un defaut de surete ou d'instrument ;
# CONVAINCANT / NON_CONVAINCANT / NON_CONCLUANT est une mesure.
#
# BO_QEMU_KVM=1 : sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_banc_irq_gpu.sh BOOTIMAGE [TOURS]}
TOURS=${2:-6}
ACCEL="-cpu max"
NOM=tcg
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
  NOM=kvm
fi
SORTIE=${BANC_SORTIE:-$PWD/banc-irq-gpu-$NOM}
rm -rf "$SORTIE"
mkdir -p "$SORTIE/scenario"
{
  echo "gpu-irq"
  for t in $(seq "$TOURS"); do
    if [ $((t % 2)) = 1 ]; then ordre="scrute irq"; else ordre="irq scrute"; fi
    for v in $ordre; do
      if [ "$v" = irq ]; then echo "gpu-irq on"; else echo "gpu-irq off"; fi
      echo "echo GPU_AB_BRAS tour=$t attente=$v"
      echo "banc-present tache"
    done
  done
  echo "gpu-irq on"
  echo "echo GPU_AB_FIN"
} > "$SORTIE/scenario/autorun"
(cd tools/userland && IMAGE="$SORTIE/banc.img" ./mkdisk.sh "$SORTIE/scenario" >/dev/null)
LOG="$SORTIE/serie.log"
: > "$LOG"
qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/banc.img" \
  -m 2048 -smp 2 -display none -no-reboot -serial file:"$LOG" -vga none -device virtio-gpu-pci &
PID=$!
LIMITE=$((SECONDS + 120 + TOURS * 2 * 60))
while kill -0 "$PID" 2>/dev/null && (( SECONDS < LIMITE )); do
  # La sortie de `echo`, pas sa trace (`+ echo GPU_AB_FIN`).
  if grep -aqE 'KERNEL PANIC|panicked at' "$LOG" \
     || sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'GPU_AB_FIN' | grep -aqv '+ echo GPU_AB_FIN'; then
    sleep 2; break
  fi
  sleep 2
done
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
python3 tools/ci/analyse_banc_irq_gpu.py "$LOG" "$TOURS" "$NOM"
