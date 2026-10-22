#!/usr/bin/env bash
# A/B : CESSION AU QUANTUM PAR CLASSE (BOUCHAUD_CESSION_PAR_CLASSE_V1) CONTRE
# LA CESSION HISTORIQUE.
#
#   tools/ci/run_ab_cession.sh BOOTIMAGE [TOURS] [SMP]
#
# Un seul demarrage, TOURS tours (6 par defaut) ; chaque tour passe
# `ordonnanceur-probe 8 1` sous `ordo-cession historique` et `classe`, ordre
# AB aux tours impairs, BA aux pairs ; le releve `ordo-cession` encadre chaque
# passe. Le demarrage finit sous `classe` (la politique par defaut).
#
# Criteres, quorum et defauts : tools/ci/analyse_ab_cession.py (declares avant
# la mesure KVM). ECHEC sur un defaut de surete ou d'instrument ;
# CONVAINCANT / NON_CONVAINCANT / NON_CONCLUANT est une mesure.
#
# BO_QEMU_KVM=1 : sous KVM (temps vole publie par l'hote, exige).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ab_cession.sh BOOTIMAGE [TOURS] [SMP]}
TOURS=${2:-6}
SMP=${3:-4}
ACCEL="-cpu max"
NOM=tcg
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host,kvm-steal-time=on"
  NOM=kvm
fi
SORTIE=${AB_SORTIE:-$PWD/ab-cession-$NOM-smp$SMP}
rm -rf "$SORTIE"
mkdir -p "$SORTIE/scenario/bin"
(cd tools/userland && OUT="$SORTIE/sondes" ./build.sh musl >/dev/null)
cp "$SORTIE/sondes/ordonnanceur-probe" "$SORTIE/scenario/bin/"
{
  for t in $(seq "$TOURS"); do
    if [ $((t % 2)) = 1 ]; then ordre="historique classe"; else ordre="classe historique"; fi
    for v in $ordre; do
      echo "echo AB_CESSION tour=$t politique=$v"
      echo "ordo-cession $v"
      echo "/bin/ordonnanceur-probe 8 1"
      echo "ordo-cession"
    done
  done
  echo "ordo-cession classe"
  echo "echo AB_CESSION_FIN"
} > "$SORTIE/scenario/autorun"
(cd tools/userland && IMAGE="$SORTIE/ab.img" ./mkdisk.sh "$SORTIE/scenario" >/dev/null)
LOG="$SORTIE/serie.log"
: > "$LOG"
qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/ab.img" \
  -m 4096 -smp "$SMP" -display none -no-reboot -serial file:"$LOG" \
  -netdev user,id=net0 -device e1000,netdev=net0 -audiodev none,id=muet -device AC97,audiodev=muet &
PID=$!
LIMITE=$((SECONDS + 180 + TOURS * 2 * 150))
while kill -0 "$PID" 2>/dev/null && (( SECONDS < LIMITE )); do
  # La sortie de `echo`, pas sa trace (`+ echo AB_CESSION_FIN`).
  if grep -aqE 'KERNEL PANIC|panicked at' "$LOG" \
     || sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'AB_CESSION_FIN' | grep -av '+ echo AB_CESSION_FIN' >/dev/null; then
    sleep 2; break
  fi
  sleep 2
done
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
python3 tools/ci/analyse_ab_cession.py "$LOG" "$TOURS" "$SMP" "$NOM"
