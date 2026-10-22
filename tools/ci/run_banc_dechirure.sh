#!/usr/bin/env bash
# P10+.3c -- BANC DE DECHIRURE du contrat de surface client
# (BOUCHAUD_GUI_SURFACE_DEUX_TAMPONS_V1).
#
#   tools/ci/run_banc_dechirure.sh BOOTIMAGE [TOURS] [SMP]
#
# Un DEMARRAGE par bras (le bureau ne lance le client qu'une fois) ; TOURS
# tours (4 par defaut), ordre alterne : v1 puis v2 aux tours impairs, v2 puis
# v1 aux pairs. Chaque demarrage installe `tools/userland/surface-probe.c` en
# `/bo-navigateur`, et :
#
#   bras v1 : `gui-surface simple`, BO_SONDE_SURFACE=v1 (le contrat d'avant :
#             un tampon reecrit des que la trame est annoncee) ;
#   bras v2 : `gui-surface double`, BO_SONDE_SURFACE=v2 (deux tampons, on
#             n'ecrit que dans un tampon libere).
#
# L'instrument `gui-surface mesure on` compte les compositions qui ont lu une
# trame non uniforme ; le bilan (`GUI_SURFACE_BILAN`) est ecrit par le noyau a
# la mort du client. Criteres : tools/ci/analyse_banc_dechirure.py.
#
# BO_QEMU_KVM=1 : sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_banc_dechirure.sh BOOTIMAGE [TOURS] [SMP]}
TOURS=${2:-4}
SMP=${3:-2}
ACCEL="-cpu max"
NOM=tcg
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
  NOM=kvm
fi
SORTIE=${AB_SORTIE:-$PWD/banc-dechirure-$NOM-smp$SMP}
rm -rf "$SORTIE"
mkdir -p "$SORTIE"
(cd tools/userland && OUT="$SORTIE/sondes" ./build.sh musl >/dev/null)
DUREE_MS=${BO_SONDE_SURFACE_DUREE_MS:-6000}

bras() {
  local tour=$1 mode=$2 offre
  if [ "$mode" = v2 ]; then offre=double; else offre=simple; fi
  local rep="$SORTIE/t$tour-$mode"
  mkdir -p "$rep/scenario"
  cp "$SORTIE/sondes/surface-probe" "$rep/scenario/bo-navigateur"
  chmod 755 "$rep/scenario/bo-navigateur"
  cat > "$rep/scenario/autorun" <<AUTORUN
echo BANC_DECHIRURE tour=$tour mode=$mode
gui-surface $offre
gui-surface mesure on
export BO_AUTOSTART_BROWSER=1
export BO_SONDE_SURFACE=$mode
export BO_SONDE_SURFACE_DUREE_MS=$DUREE_MS
desktop
AUTORUN
  (cd tools/userland && IMAGE="$rep/disque.img" ./mkdisk.sh "$rep/scenario" >/dev/null)
  local log="$rep/serie.log"
  : > "$log"
  qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$rep/disque.img" \
    -m 4096 -smp "$SMP" -display none -no-reboot -serial file:"$log" \
    -netdev user,id=net0 -device e1000,netdev=net0 -audiodev none,id=muet -device AC97,audiodev=muet &
  local pid=$!
  local limite=$((SECONDS + 600))
  # Le bilan est ecrit par le NOYAU a la mort du client : aucune trace du
  # shell ne le porte (l'echo `+ ...` ne peut pas le simuler).
  while kill -0 "$pid" 2>/dev/null && (( SECONDS < limite )); do
    if grep -aqE 'KERNEL PANIC|panicked at|GUI_SURFACE_BILAN ' "$log"; then
      sleep 2; break
    fi
    sleep 2
  done
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  cat "$log" >> "$SORTIE/serie.log"
}

: > "$SORTIE/serie.log"
for t in $(seq "$TOURS"); do
  if [ $((t % 2)) = 1 ]; then ordre="v1 v2"; else ordre="v2 v1"; fi
  for m in $ordre; do
    bras "$t" "$m"
  done
done
python3 tools/ci/analyse_banc_dechirure.py "$SORTIE/serie.log" "$TOURS" "$SMP" "$NOM"
