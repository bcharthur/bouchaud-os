#!/usr/bin/env bash
# P10+.1 -- PANNES DE L'ATTENTE PAR INTERRUPTION (BOUCHAUD_VIRTIO_GPU_IRQ_V1).
#
#   tools/ci/run_virtio_gpu_irq_pannes.sh BOOTIMAGE_A_OPTION
#
# Noyau compile avec `banc-virtio-gpu-irq-pannes`. Le bureau tourne sur
# l'ecran virtio-gpu seul, attente par interruption (`gpu-irq on`), et subit
# dans l'ordre :
#
#   1. l'entree MSI-X masquee pendant dix attentes : les interruptions sont
#      PERDUES ; chaque reponse doit etre trouvee par la relecture de secours
#      (`vus_par_relecture` >= 10, une par attente masquee), sans echec de presentation ;
#   2. des interruptions parasites sur le vecteur, EN VOL (avant chaque
#      attente de la fenetre et pendant le sommeil : preuve, elles peuvent
#      fusionner avec la vraie MSI-X) et HORS VOL (apres chaque achevement de
#      la fenetre, MSI-X servie : au moins six, chacune exactement un passage
#      du gestionnaire sans progres) ; aucune presentation refusee -- elles
#      n'achevent rien (la preuve deterministe du reveil parasite pendant un
#      sommeil est le test hote `interruption_parasite_n_acheve_rien`) ;
#   3. une attente dont la reponse n'est jamais vue : elle finit a son
#      echeance (`echues=1`), la file est cassee, le scanout abandonne au
#      premier echec, le compositeur tourne encore. Aucune panique.
#
# BO_QEMU_KVM=1 : meme demarrage sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
BOOT=${1:?usage: run_virtio_gpu_irq_pannes.sh BOOTIMAGE}
SORTIE=${PANNES_SORTIE:-$PWD/virtio-irq-pannes}
rm -rf "$SORTIE"
mkdir -p "$SORTIE/scenario"
printf 'gpu-irq on\ndesktop\n' > "$SORTIE/scenario/autorun"
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
LIMITE=$((SECONDS + 300))
while kill -0 "$PID" 2>/dev/null && (( SECONDS < LIMITE )); do
  grep -aqE 'KERNEL PANIC|panicked at' "$LOG" && break
  if grep -aq 'VIRTIO_GPU_SCANOUT_ABANDON' "$LOG" \
     && [ "$(apres_abandon | grep -c .)" -ge 2 ]; then
    sleep 2; break
  fi
  sleep 1
done
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
propre() { sed -E 's/\x1b\[[0-9;]*m//g' "$LOG"; }
propre | grep -aE 'VIRTIO_GPU_(IRQ|INJECTION|FILE|SCANOUT)|\[GUI-VIRTIO-GPU\]|KERNEL PANIC|panicked' | sed 's/^/  /' | awk 'NR<=16' || true
# Le dernier releve, apres l'abandon.
APRES=$(apres_abandon | tail -1 | grep -ao '\[GUI-VIRTIO-GPU\].*' || true)
val() { echo "$1" | grep -oE " $2=[0-9]+" | awk 'NR<=1' | cut -d= -f2; }

echecs=0
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs=$((echecs + 1)); fi; }
exige "MSI-X arme (achevement par interruption)" grep -aq 'VIRTIO_GPU_IRQ msix vecteur=0x50' "$LOG"
exige "le scanout virtio-gpu etait actif" grep -aq 'BOUCHAUD_VIRTIO_GPU_SCANOUT_ACTIF' "$LOG"
exige "les trois injections ont eu lieu" bash -c "grep -aq 'VIRTIO_GPU_INJECTION irq_masquee' '$LOG' && grep -aq 'VIRTIO_GPU_INJECTION irq_demasquee' '$LOG' && grep -aq 'VIRTIO_GPU_INJECTION irq_parasites' '$LOG' && grep -aq 'VIRTIO_GPU_INJECTION reponse_jamais_vue' '$LOG'"
echo "  preuve  dernier releve apres l'abandon : $APRES"
exige "attente par interruption effective (attentes dormantes > 0)" test "$(val "$APRES" dormantes)" -gt 0 2>/dev/null
# La reponse d'une attente masquee n'est visible qu'apres un premier sommeil
# (injection deterministe, run 38704459647) : chacune des dix est retrouvee
# par la relecture.
exige "interruptions perdues : les dix reponses masquees trouvees par la relecture (vus_par_relecture >= 10)" test "$(val "$APRES" vus_par_relecture)" -ge 10 2>/dev/null
# Deux sortes de parasites. EN VOL (pendant un sommeil) : elle reveille
# l'attente, qui doit se rendormir -- mais elle peut fusionner avec la vraie
# MSI-X de la reponse (un vecteur n'a qu'une place dans l'IRR : run
# 38732049308, KVM, 9 envoyees pour 8 passages sans progres), le compte n'est
# donc qu'une preuve. HORS VOL (apres l'achevement, MSI-X servie) : rien ne
# peut fusionner, chacune doit produire exactement un passage sans progres.
FIN=$(propre | grep -aoE 'irq_parasites_fin envoyees=[0-9]+ hors_vol=[0-9]+ hors_vol_sans_progres=[0-9]+' | awk 'NR<=1')
ENVOYEES=$(echo "$FIN" | grep -oE 'envoyees=[0-9]+' | cut -d= -f2)
HORS_VOL=$(echo "$FIN" | grep -oE 'hors_vol=[0-9]+' | cut -d= -f2)
HORS_VOL_VUES=$(echo "$FIN" | grep -oE 'hors_vol_sans_progres=[0-9]+' | cut -d= -f2)
echo "  preuve  parasites en vol=${ENVOYEES:-?} ; reveils sans progres=$(val "$APRES" reveils_sans_progres) ; irq sans progres=$(val "$APRES" irq_sans_progres) (une parasite en vol peut fusionner avec la reponse)"
exige "parasites hors vol : au moins six, chacune un passage du gestionnaire sans progres (${HORS_VOL_VUES:-?}/${HORS_VOL:-?})" \
  bash -c "[ '${HORS_VOL:-0}' -ge 6 ] && [ '${HORS_VOL_VUES:--1}' = '${HORS_VOL:-0}' ]" 2>/dev/null
exige "une seule attente echue (la reponse jamais vue)" test "$(val "$APRES" echues)" = 1
exige "un seul echec de presentation (la reponse jamais vue), au moins 5 acceptees avant" \
  bash -c "[ '$(val "$APRES" presents_echec)' = 1 ] && [ '$(val "$APRES" presents_ok)' -ge 5 ]" 2>/dev/null
exige "le scanout est abandonne (file cassee) au premier echec" grep -aq 'VIRTIO_GPU_SCANOUT_ABANDON raison=file-sans-reponse echecs_consecutifs=1' "$LOG"
exige "le compositeur tourne encore apres l'abandon (deux releves entiers)" test "$(apres_abandon | grep -c .)" -ge 2
exige "aucune panique" bash -c "! grep -aqE 'KERNEL PANIC|panicked at' '$LOG'"
if [ "$echecs" -eq 0 ]; then
  echo "VIRTIO_GPU_IRQ_PANNES_BANC_OK"
else
  echo "VIRTIO_GPU_IRQ_PANNES_BANC_ECHEC n=$echecs"
  exit 1
fi
