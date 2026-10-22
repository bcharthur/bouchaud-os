#!/usr/bin/env bash
# P10 -- PANNE : ATTACH_BACKING SANS REPONSE (BOUCHAUD_VIRTIO_GPU_DMA_ETAT_V1).
#
#   tools/ci/run_virtio_gpu_dma_panne.sh BOOTIMAGE_AVEC_banc-virtio-gpu-sans-reponse
#
# Le noyau, compile avec l'option `banc-virtio-gpu-sans-reponse`, ENVOIE le
# `RESOURCE_ATTACH_BACKING` du scanout du bureau puis declare l'attente
# echue. QEMU, lui, a bien recu la requete et retenu l'adresse : c'est le cas
# ou rendre la memoire a l'allocateur la livrerait a un peripherique qui peut
# encore y lire ou y ecrire.
#
# Exige : l'injection a eu lieu ; le scanout est refuse a l'etape
# d'attache ; la memoire est CONSERVEE (etat Inconnue) ; le bureau se replie
# (present() inactif) ; aucune panique.
#
# Avant le correctif, `attachee` restait faux et la memoire etait rendue :
# la ligne `memoire_conservee` n'apparaissait pas -- ce banc rougissait.
#
# BO_QEMU_KVM=1 : meme demarrage sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
BOOT=${1:?usage: run_virtio_gpu_dma_panne.sh BOOTIMAGE}
SORTIE=${PANNE_SORTIE:-$PWD/virtio-dma-panne}
rm -rf "$SORTIE"
mkdir -p "$SORTIE/scenario"
printf 'desktop\n' > "$SORTIE/scenario/autorun"
(cd tools/userland && IMAGE="$SORTIE/bureau.img" ./mkdisk.sh "$SORTIE/scenario" >/dev/null)
LOG="$SORTIE/serie.log"
: > "$LOG"
qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/bureau.img" \
  -m 2048 -smp 2 -display none -no-reboot -vga none -device virtio-gpu-pci \
  -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 240))
while kill -0 "$PID" 2>/dev/null && (( SECONDS < LIMITE )); do
  grep -aqE 'KERNEL PANIC|panicked at' "$LOG" && break
  grep -aq '\[gui\] window manager demarre' "$LOG" && { sleep 3; break; }
  sleep 1
done
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -aE 'VIRTIO_GPU|\[gfx\]|KERNEL PANIC|panicked' | sed 's/^/  /' | head -20 || true

echecs=0
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs=$((echecs + 1)); fi; }
exige "l'autotest 2D est passe (le peripherique repond)" grep -aq 'BOUCHAUD_VIRTIO_GPU_2D_AUTOTEST_OK' "$LOG"
exige "l'injection a eu lieu (requete envoyee, attente declaree echue)" grep -aq 'VIRTIO_GPU_INJECTION attache_bureau_sans_reponse' "$LOG"
exige "le scanout est refuse a l'etape d'attache" grep -aq 'VIRTIO_GPU_SCANOUT refuse raison=bureau-attache' "$LOG"
exige "la memoire du bureau est CONSERVEE (etat Inconnue)" grep -aq 'VIRTIO_GPU_DMA memoire_conservee quoi=bureau .* etat=Inconnue' "$LOG"
exige "le scanout n'est pas declare actif" bash -c "! grep -aq 'BOUCHAUD_VIRTIO_GPU_SCANOUT_ACTIF' '$LOG'"
exige "le bureau se replie et le dit" grep -aq 'present() inactif (scanout=virtio-gpu-2d)' "$LOG"
exige "aucune panique" bash -c "! grep -aqE 'KERNEL PANIC|panicked at' '$LOG'"
if [ "$echecs" -eq 0 ]; then
  echo "VIRTIO_GPU_DMA_PANNE_BANC_OK"
else
  echo "VIRTIO_GPU_DMA_PANNE_BANC_ECHEC n=$echecs"
  exit 1
fi
