#!/usr/bin/env bash
# P13+ -- A/B : REVEIL A L'ECHEANCE EXACTE AU REPOS (BOUCHAUD_REVEIL_PRECIS_V1).
#
#   tools/ci/run_ab_reveil_precis.sh BOOTIMAGE [TOURS] [SMP]
#
# Un seul demarrage, TOURS tours (4 par defaut) ; chaque tour passe
# `ordonnanceur-probe 8 1` sous `reveil-precis off` et `on`, ordre alterne
# (AB aux tours impairs, BA aux pairs). Le releve `reveil-precis` encadre
# chaque passe. Le demarrage finit sous `off` (la politique par defaut).
#
# Criteres et verdicts : tools/ci/analyse_ab_reveil_precis.py (declares
# avant la mesure). ECHEC sur un defaut de surete ou d'instrument ;
# CONVAINCANT / NON_CONVAINCANT / NON_CONCLUANT est une mesure.
#
# BO_QEMU_KVM=1 : sous KVM (temps vole publie par l'hote).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ab_reveil_precis.sh BOOTIMAGE [TOURS] [SMP]}
TOURS=${2:-4}
SMP=${3:-1}
ACCEL="-cpu max"
NOM=tcg
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host,kvm-steal-time=on"
  NOM=kvm
fi
SORTIE=${AB_SORTIE:-$PWD/ab-reveil-precis-$NOM-smp$SMP}
rm -rf "$SORTIE"
mkdir -p "$SORTIE/scenario/bin"
(cd tools/userland && OUT="$SORTIE/sondes" ./build.sh musl >/dev/null)
cp "$SORTIE/sondes/ordonnanceur-probe" "$SORTIE/scenario/bin/"
{
  for t in $(seq "$TOURS"); do
    if [ $((t % 2)) = 1 ]; then ordre="off on"; else ordre="on off"; fi
    for v in $ordre; do
      echo "echo AB_PRECIS tour=$t reveil=$v"
      echo "reveil-precis $v"
      echo "/bin/ordonnanceur-probe 8 1"
      echo "reveil-precis"
    done
  done
  echo "reveil-precis off"
  echo "echo AB_PRECIS_FIN"
} > "$SORTIE/scenario/autorun"
(cd tools/userland && IMAGE="$SORTIE/ab.img" ./mkdisk.sh "$SORTIE/scenario" >/dev/null)
LOG="$SORTIE/serie.log"
: > "$LOG"
qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/ab.img" \
  -m 4096 -smp "$SMP" -display none -no-reboot -serial file:"$LOG" \
  -netdev user,id=net0 -device e1000,netdev=net0 -audiodev none,id=muet -device AC97,audiodev=muet &
PID=$!
LIMITE=$((SECONDS + 180 + TOURS * 2 * 300))
while kill -0 "$PID" 2>/dev/null && (( SECONDS < LIMITE )); do
  if grep -aqE 'KERNEL PANIC|panicked at' "$LOG" \
     || sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'AB_PRECIS_FIN' | grep -aqv '+ echo AB_PRECIS_FIN'; then
    sleep 2; break
  fi
  sleep 2
done
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
python3 tools/ci/analyse_ab_reveil_precis.py "$LOG" "$TOURS" "$SMP" "$NOM"
