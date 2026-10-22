#!/usr/bin/env bash
# P10+.2b -- A/B : TAMPON ARRIERE + COPIE CONTRE COMPOSITION DIRECTE DANS LA
# MEMOIRE DU SCANOUT VIRTIO-GPU (BOUCHAUD_VIRTIO_GPU_TAMPONS_V1).
#
#   tools/ci/run_banc_tampon.sh BOOTIMAGE [TOURS]
#
# Un demarrage `-vga none -device virtio-gpu-pci`, TOURS tours (6 par defaut)
# de `banc-present tache` sous `gpu-tampon copie` et `gpu-tampon direct`,
# ordre AB aux tours impairs, BA aux pairs (chaque banc refait `enter()`, qui
# prend la politique). Le demarrage finit sous `direct` (la politique par
# defaut depuis l'A/B KVM CONVAINCANT de 77e0e5de).
#
# Criteres declares dans tools/ci/analyse_banc_tampon.py. ECHEC sur un defaut
# de surete (copie en direct, acces pendant un transfert, presentation en
# echec, scanout abandonne, panique) ; CONVAINCANT / NON_CONVAINCANT /
# NON_CONCLUANT est une mesure.
#
# BO_QEMU_KVM=1 : sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_banc_tampon.sh BOOTIMAGE [TOURS]}
TOURS=${2:-6}
ACCEL="-cpu max"
NOM=tcg
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
  NOM=kvm
fi
SORTIE=${BANC_SORTIE:-$PWD/banc-tampon-$NOM}
rm -rf "$SORTIE"
mkdir -p "$SORTIE/scenario"
{
  for t in $(seq "$TOURS"); do
    if [ $((t % 2)) = 1 ]; then ordre="copie direct"; else ordre="direct copie"; fi
    for v in $ordre; do
      echo "echo AB_TAMPON tour=$t tampon=$v"
      echo "gpu-tampon $v"
      echo "banc-present tache"
      echo "gpu-tampon"
    done
  done
  echo "gpu-tampon direct"
  echo "echo AB_TAMPON_FIN"
} > "$SORTIE/scenario/autorun"
(cd tools/userland && IMAGE="$SORTIE/banc.img" ./mkdisk.sh "$SORTIE/scenario" >/dev/null)
LOG="$SORTIE/serie.log"
: > "$LOG"
qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/banc.img" \
  -m 2048 -smp 2 -display none -no-reboot -serial file:"$LOG" -vga none -device virtio-gpu-pci &
PID=$!
LIMITE=$((SECONDS + 180 + TOURS * 2 * 90))
while kill -0 "$PID" 2>/dev/null && (( SECONDS < LIMITE )); do
  # La sortie de `echo`, pas sa trace (`+ echo AB_TAMPON_FIN`).
  if grep -aqE 'KERNEL PANIC|panicked at|\[BANC-PRESENT\] ECHEC' "$LOG" \
     || sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'AB_TAMPON_FIN' | grep -av '+ echo AB_TAMPON_FIN' >/dev/null; then
    sleep 2; break
  fi
  sleep 2
done
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
python3 tools/ci/analyse_banc_tampon.py "$LOG" "$TOURS" "$NOM"
