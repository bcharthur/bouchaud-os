#!/usr/bin/env bash
# P10 -- PANNE : REPONSE ETRANGERE SUR LA FILE DE CONTROLE
# (BOUCHAUD_VIRTIO_GPU_BARRIERE_V1).
#
#   tools/ci/run_virtio_gpu_reponse_etrangere.sh BOOTIMAGE_AVEC_banc-virtio-gpu-reponse-etrangere
#
# Toute commande virtio-gpu part avec VIRTIO_GPU_FLAG_FENCE ; sa reponse doit
# rendre la meme barriere, sur une seule entree utilisee de la bonne chaine.
# Le noyau, compile avec l'option `banc-virtio-gpu-reponse-etrangere`, juge
# le premier transfert du bureau contre une barriere qui n'est pas la sienne :
# ce que verrait le pilote si le peripherique rendait la reponse d'une autre
# commande (achevement desordonne ou tardif).
#
# Exige : le scanout etait actif ; l'injection a eu lieu ; la violation est
# nommee (`barriere-differente` sur la commande 0x105) et chiffree (barriere
# recue = attendue - 1 : la reponse est bien celle d'une AUTRE commande) ; le
# scanout est abandonne (file cassee) apres UN seul echec -- aucune commande
# ne repart sur une file cassee ; aucune panique ; le bureau continue (deux
# releves du compositeur APRES l'abandon, scanout inactif).
#
# BO_QEMU_KVM=1 : meme demarrage sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
BOOT=${1:?usage: run_virtio_gpu_reponse_etrangere.sh BOOTIMAGE}
SORTIE=${ETRANGERE_SORTIE:-$PWD/virtio-reponse-etrangere}
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
# P10_TELEMETRIE_COMPLETE_V1 : les releves du compositeur APRES l'abandon,
# ENTIERS seulement (QEMU est tue au fil de l'eau ; run 40715886448 : ligne
# coupee a `us_par_present=250 ac`).
TELEMETRIE='\[GUI-VIRTIO-GPU\] presents_ok=.* regions_differees=[0-9]+$'
apres_abandon() { sed -n '/VIRTIO_GPU_SCANOUT_ABANDON/,$p' "$LOG" > "$SORTIE/apres-abandon.log"
  python3 tools/ci/ligne_complete.py "$SORTIE/apres-abandon.log" "$TELEMETRIE" --toutes || true; }
LIMITE=$((SECONDS + 240))
while kill -0 "$PID" 2>/dev/null && (( SECONDS < LIMITE )); do
  grep -aqE 'KERNEL PANIC|panicked at' "$LOG" && break
  # Apres l'abandon, attendre deux releves du compositeur : il doit tourner.
  if grep -aq 'VIRTIO_GPU_SCANOUT_ABANDON' "$LOG" \
     && [ "$(apres_abandon | grep -c .)" -ge 2 ]; then
    sleep 2; break
  fi
  sleep 1
done
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -aE 'VIRTIO_GPU_(INJECTION|FILE|SCANOUT)|\[GUI-VIRTIO-GPU\]|KERNEL PANIC|panicked' | sed 's/^/  /' | awk 'NR<=12' || true

echecs=0
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs=$((echecs + 1)); fi; }
exige "le scanout virtio-gpu etait actif" grep -aq 'BOUCHAUD_VIRTIO_GPU_SCANOUT_ACTIF' "$LOG"
exige "l'injection a eu lieu" grep -aq 'VIRTIO_GPU_INJECTION reponse_etrangere' "$LOG"
exige "la violation est nommee (barriere differente, transfert 0x105)" grep -aq 'VIRTIO_GPU_FILE violation=barriere-differente commande=0x105' "$LOG"
ecart_barriere() {
  local l b a
  l=$(sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -ao 'VIRTIO_GPU_FILE violation=barriere-differente.*' | awk 'NR<=1')
  b=$(sed -nE 's/.* barriere=([0-9]+) .*/\1/p' <<<"$l")
  a=$(sed -nE 's/.* attendue=([0-9]+).*/\1/p' <<<"$l")
  echo "  preuve  barriere_recue=${b:-?} barriere_attendue=${a:-?}"
  [ -n "$b" ] && [ -n "$a" ] && [ $((a - b)) -eq 1 ]
}
exige "la reponse rendue porte la barriere d'une autre commande (attendue - 1)" ecart_barriere
exige "le scanout est abandonne (file cassee) au premier echec" grep -aq 'VIRTIO_GPU_SCANOUT_ABANDON raison=file-sans-reponse echecs_consecutifs=1' "$LOG"
exige "aucune commande apres l'abandon (un seul echec de presentation)" \
  bash -c "[ -n \"\$1\" ] && ! printf '%s\n' \"\$1\" | grep -avE 'presents_ok=0 presents_echec=1 .* actif=0 '" _ "$(apres_abandon)"
exige "le compositeur tourne encore apres l'abandon (deux releves entiers)" test "$(apres_abandon | grep -c .)" -ge 2
exige "aucune panique" bash -c "! grep -aqE 'KERNEL PANIC|panicked at' '$LOG'"
if [ "$echecs" -eq 0 ]; then
  echo "VIRTIO_GPU_REPONSE_ETRANGERE_BANC_OK"
else
  echo "VIRTIO_GPU_REPONSE_ETRANGERE_BANC_ECHEC n=$echecs"
  exit 1
fi
