#!/usr/bin/env bash
# ORDONNANCEUR-PROBE, ATTRIBUTION DES RETARDS (BOUCHAUD_ATTRIBUTION_REVEIL_V1).
#
#   tools/ci/run_ordonnanceur_attribution.sh BOOTIMAGE [PASSES] [SMP]
#
# Lance `ordonnanceur-probe 8` PASSES fois de suite (3 par defaut) dans un
# seul demarrage, a SMP coeurs (1 par defaut). Le critere de la sonde est
# INCHANGE ; chaque passe doit rendre `RESULTAT : 0`. Le banc imprime, pour
# chaque phase, ce que le noyau dit du pire reveil : balayage tardif, attente
# d'election, tics perdus, section masquee avant le pire ecart de tic, et qui
# tenait le coeur.
#
# BO_QEMU_KVM=1 : sous KVM. C'est le temoin : sous TCG, l'endurance a montre
# des tics du PIT perdus HORS de l'invite (ecarts de 10 a 80 ms finis en
# ring 3 ou a l'idle, sans section masquee) ; sous KVM ils ne doivent pas
# exister, et la sonde ne doit pas rougir.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ordonnanceur_attribution.sh BOOTIMAGE [PASSES] [SMP]}
PASSES=${2:-3}
SMP=${3:-1}
ACCEL="-cpu max"
NOM=tcg
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
  NOM=kvm
fi
SORTIE=${ORDO_SORTIE:-$PWD/ordonnanceur-attribution-$NOM}
rm -rf "$SORTIE"
mkdir -p "$SORTIE/scenario/bin"
(cd tools/userland && OUT="$SORTIE/sondes" ./build.sh musl >/dev/null)
cp "$SORTIE/sondes/ordonnanceur-probe" "$SORTIE/scenario/bin/"
{
  echo "echo ORDO_ATTRIBUTION_DEBUT"
  for _ in $(seq "$PASSES"); do echo "/bin/ordonnanceur-probe 8"; done
  echo "echo ORDO_ATTRIBUTION_FIN"
} > "$SORTIE/scenario/autorun"
(cd tools/userland && IMAGE="$SORTIE/ordo.img" ./mkdisk.sh "$SORTIE/scenario" >/dev/null)
LOG="$SORTIE/serie.log"
: > "$LOG"
qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$SORTIE/ordo.img" \
  -m 4096 -smp "$SMP" -display none -no-reboot -serial file:"$LOG" \
  -netdev user,id=net0 -device e1000,netdev=net0 -audiodev none,id=muet -device AC97,audiodev=muet &
PID=$!
LIMITE=$((SECONDS + 120 + PASSES * 120))
while kill -0 "$PID" 2>/dev/null && (( SECONDS < LIMITE )); do
  grep -aqE 'ORDO_ATTRIBUTION_FIN|KERNEL PANIC|panicked at' "$LOG" && { sleep 2; break; }
  sleep 2
done
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
propre() { sed -E 's/\x1b\[[0-9;]*m//g; s/^(\[[^]]*\]){2}\[FPS:[^]]*\] //' "$1"; }
propre "$LOG" | grep -aE '^  (au repos|sous charge)|^  attribution|^  - |^RESULTAT|\[POINTS-COOP\]' | sed 's/^/  /' || true

echecs=0
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs=$((echecs + 1)); fi; }
exige "les $PASSES passes sont allees au bout" grep -aq 'ORDO_ATTRIBUTION_FIN' "$LOG"
VERTES=$(propre "$LOG" | grep -ac '^RESULTAT : 0 verification' || true)
ATTRIBUEES=$(propre "$LOG" | grep -ac '^  attribution .* : reveils=' || true)
exige "chaque passe rend RESULTAT : 0 (critere inchange) : $VERTES/$PASSES" test "$VERTES" = "$PASSES"
exige "le noyau a attribue chaque phase : $ATTRIBUEES/$((PASSES * 3))" test "$ATTRIBUEES" = "$((PASSES * 3))"
exige "aucune panique" bash -c "! grep -aqE 'KERNEL PANIC|panicked at' '$LOG'"
if [ "$echecs" -eq 0 ]; then
  echo "ORDONNANCEUR_ATTRIBUTION_OK passes=$PASSES smp=$SMP accel=$NOM"
else
  echo "ORDONNANCEUR_ATTRIBUTION_ECHEC n=$echecs passes=$PASSES smp=$SMP accel=$NOM"
  exit 1
fi
