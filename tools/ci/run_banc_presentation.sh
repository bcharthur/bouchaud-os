#!/usr/bin/env bash
# P10 -- BGA CONTRE VIRTIO-GPU 2D, MEME CHARGE (BOUCHAUD_BANC_PRESENTATION_V1).
#
#   tools/ci/run_banc_presentation.sh BOOTIMAGE
#
# Deux demarrages, meme scenario (`banc-present`) : ecran BGA (`-vga std`),
# puis ecran virtio-gpu seul (`-vga none -device virtio-gpu-pci`). Le noyau
# presente la meme suite de trames par categorie -- plein ecran, bande de
# 30 lignes, tuile 64x64 -- et chronometre chaque presentation (copie vers
# l'ecran, plus transfert + flush et leurs reponses sous virtio-gpu).
#
# Exige, pour chaque ecran : les trois categories mesurees avec leur nombre de
# presentations ; sous virtio-gpu, toutes acceptees par le peripherique (aucun
# echec) et le scanout toujours actif a la fin ; aucune panique. Le banc
# imprime le tableau cote a cote. Il ne juge PAS qui est le plus rapide : il
# mesure. Scanout 2D seulement -- aucune acceleration, aucune 3D.
#
# BO_QEMU_KVM=1 : memes demarrages sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_banc_presentation.sh BOOTIMAGE}
ACCEL="-cpu max"
NOM=tcg
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
  NOM=kvm
fi
SORTIE=${BANC_SORTIE:-$PWD/banc-presentation-$NOM}
rm -rf "$SORTIE"
mkdir -p "$SORTIE/scenario"
# Deux bancs dans le meme demarrage : le second `enter()` reprend le scanout
# (BOUCHAUD_VIRTIO_GPU_SCANOUT_CYCLE_V1 -- il echouait un demarrage sur deux).
printf 'banc-present\nbanc-present\necho BANC_PRESENT_FIN\n' > "$SORTIE/scenario/autorun"
(cd tools/userland && IMAGE="$SORTIE/banc.img" ./mkdisk.sh "$SORTIE/scenario" >/dev/null)

demarre() {
  local ecran=$1; shift
  local log="$SORTIE/serie-$ecran.log"
  cp "$SORTIE/banc.img" "$SORTIE/banc-$ecran.img"
  : > "$log"
  qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/banc-$ecran.img" \
    -m 2048 -smp 2 -display none -no-reboot -serial file:"$log" "$@" &
  local pid=$!
  local limite=$((SECONDS + 300))
  while kill -0 "$pid" 2>/dev/null && (( SECONDS < limite )); do
    # La SORTIE de `echo BANC_PRESENT_FIN`, pas l'echo de la commande.
    if grep -aqE '\[BANC-PRESENT\] ECHEC|KERNEL PANIC|panicked at' "$log" \
       || sed -E 's/\x1b\[[0-9;]*m//g' "$log" | grep -a 'BANC_PRESENT_FIN' | grep -av '+ echo BANC_PRESENT_FIN' >/dev/null; then
      sleep 1; break
    fi
    sleep 1
  done
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  rm -f "$SORTIE/banc-$ecran.img"
}
demarre bga -vga std
demarre virtio -vga none -device virtio-gpu-pci

propre() { sed -E 's/\x1b\[[0-9;]*m//g' "$1" | grep -aoE '\[BANC-PRESENT\].*' || true; }
echo "  BGA contre virtio-gpu 2D ($NOM), cout d'une presentation (copie + commandes) :"
printf '  %-8s %-10s %10s %10s %10s %10s %8s\n' categorie rect "bga moy" "virtio moy" "bga p95" "virtio p95" "rapport"
for cat in plein bande tuile; do
  b=$(propre "$SORTIE/serie-bga.log" | grep " categorie=$cat " | head -1 || true)
  v=$(propre "$SORTIE/serie-virtio.log" | grep " categorie=$cat " | head -1 || true)
  val() { echo "$1" | grep -oE " $2=[0-9x]+" | cut -d= -f2; }
  bm=$(val "$b" moyen_us); vm=$(val "$v" moyen_us); bp=$(val "$b" p95_us); vp=$(val "$v" p95_us)
  rect=$(val "$b" rect)
  rapport=$(python3 -c "import sys; b,v=sys.argv[1:]; print(f'{int(v)/max(int(b),1):.1f}x' if b and v else '-')" "${bm:-0}" "${vm:-0}")
  printf '  %-8s %-10s %10s %10s %10s %10s %8s\n' "$cat" "${rect:--}" "${bm:--}" "${vm:--}" "${bp:--}" "${vp:--}" "$rapport"
done
propre "$SORTIE/serie-bga.log" | sed 's/^/  /'
propre "$SORTIE/serie-virtio.log" | sed 's/^/  /'

echecs=0
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs=$((echecs + 1)); fi; }
for ecran in bga virtio; do
  log="$SORTIE/serie-$ecran.log"
  attendu=$([ "$ecran" = bga ] && echo bga || echo virtio-gpu-2d)
  for spec in "plein 40" "bande 200" "tuile 400"; do
    set -- $spec
    exige "$ecran : categorie $1 mesuree sur l'ecran $attendu ($2 presentations)" \
      bash -c "sed -E 's/\x1b\[[0-9;]*m//g' '$log' | grep -aqE '\[BANC-PRESENT\] ecran=$attendu categorie=$1 rect=[0-9]+x[0-9]+ n=$2 '"
  done
  exige "$ecran : aucune panique" bash -c "! grep -aqE 'KERNEL PANIC|panicked at' '$log'"
done
exige "virtio : 640 presentations acceptees, aucun echec, scanout actif a la fin -- deux bancs de suite" \
  bash -c "[ \"\$(sed -E 's/\x1b\[[0-9;]*m//g' '$SORTIE/serie-virtio.log' | grep -acE '\[BANC-PRESENT\] ecran=virtio-gpu-2d fin presentations_virtio_ok=640 echecs_virtio=0 actif=1')\" = 2 ]"
exige "virtio : le second banc reprend la ressource du bureau (aucune recreation)" \
  grep -aq 'BOUCHAUD_VIRTIO_GPU_SCANOUT_REPRIS 1280x720 ressource=1' "$SORTIE/serie-virtio.log"
exige "bga : deux bancs de suite" \
  bash -c "[ \"\$(sed -E 's/\x1b\[[0-9;]*m//g' '$SORTIE/serie-bga.log' | grep -acE '\[BANC-PRESENT\] ecran=bga fin ')\" = 2 ]"
if [ "$echecs" -eq 0 ]; then
  echo "BANC_PRESENTATION_OK accel=$NOM"
else
  echo "BANC_PRESENTATION_ECHEC n=$echecs accel=$NOM"
  exit 1
fi
