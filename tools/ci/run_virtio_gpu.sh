#!/usr/bin/env bash
# PREMIER COMPOSANT DE P10 : LE TRANSPORT VIRTIO-PCI (BOUCHAUD_VIRTIO_PCI_V1).
#
#   tools/ci/run_virtio_gpu.sh BOOTIMAGE
#
# Deux demarrages sans disque de donnees :
#   1. avec `-device virtio-gpu-pci` : le noyau negocie VIRTIO_F_VERSION_1,
#      monte la controlq et fait un aller-retour GET_DISPLAY_INFO
#      (BOUCHAUD_VIRTIO_GPU_OK, au moins un ecran actif) ;
#   2. sans : une seule ligne `VIRTIO_GPU absent`, rien d'autre ne change.
# Ce n'est pas un pilote d'affichage et rien n'est accelere : P10 reste
# BLOCKED (docs/ladybird/P10_GPU_AUDIT.md). virtio-gpu (QEMU) n'est pas le
# GPU de la Trigkey. BO_QEMU_KVM=1 : memes demarrages sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_virtio_gpu.sh BOOTIMAGE}
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
echo "VIRTIO_ACCEL ${ACCEL}"

demarre() {
  local nom=$1 log="serie-virtio-$1.log"; shift
  : > "$log"
  # shellcheck disable=SC2086
  qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -m 2048 -smp 2 \
    -display none -no-reboot "$@" -serial file:"$log" &
  local pid=$! limite=$((SECONDS + 120))
  while kill -0 "$pid" 2>/dev/null && (( SECONDS < limite )); do
    grep -aqE 'BOUCHAUD_VIRTIO_GPU_OK|VIRTIO_GPU_ECHEC|VIRTIO_GPU absent|KERNEL PANIC|panicked at' "$log" && { sleep 2; break; }
    sleep 1
  done
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$log" \
    | grep -aE 'VIRTIO|KERNEL PANIC|panicked' | sed "s/^/  [$nom] /" || true
}

demarre avec -device virtio-gpu-pci
demarre sans

echo "== verdict =="
echecs=0
verifie() { if grep -aqE "$2" "$1"; then echo "  ok      $3"; else echo "  ECHEC   $3"; echecs=$((echecs + 1)); fi; }
verifie serie-virtio-avec.log 'VIRTIO_GPU_INIT .*retenues=0x0000000100000000' "VERSION_1 seule negociee"
verifie serie-virtio-avec.log 'VIRTIO_GPU_DISPLAY_INFO ecrans_actifs=[1-9]' "GET_DISPLAY_INFO : au moins un ecran actif"
verifie serie-virtio-avec.log 'BOUCHAUD_VIRTIO_GPU_OK' "aller-retour sur la controlq"
verifie serie-virtio-sans.log 'VIRTIO_GPU absent' "sans le peripherique : une ligne, rien d'autre"
if grep -aqE 'KERNEL PANIC|panicked at|VIRTIO_GPU_ECHEC' serie-virtio-avec.log serie-virtio-sans.log; then
  echo "  ECHEC   panique ou echec virtio"; echecs=$((echecs + 1))
else
  echo "  ok      aucune panique"
fi
if [ "$echecs" -eq 0 ]; then echo "VIRTIO_PCI_BANC_OK"; else echo "VIRTIO_PCI_BANC_ECHEC n=$echecs"; exit 1; fi
