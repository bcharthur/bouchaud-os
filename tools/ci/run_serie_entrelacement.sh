#!/usr/bin/env bash
# LIGNES ENTIERES SUR LA CONSOLE SERIE (BOUCHAUD_SERIE_ECRITURE_ATOMIQUE_V1).
#
#   tools/ci/run_serie_entrelacement.sh BOOTIMAGE [ECRIVAINS LIGNES PROCESSUS CHARGE]
#
# serie-entrelacement-probe : des ecrivains emettent des lignes longues et
# verifiables pendant qu'un brasseur fait naitre et mourir des processus (le
# noyau ecrit alors depuis d'autres coeurs). Toutes les lignes doivent sortir
# intactes, et le bilan serie de fin d'autorun doit dire 0 emission a la borne
# (une ligne emise sans le jeton) et 0 reentrance.
#
# Run 38482972584 (KVM) : `pid=1\x1b6[ 90kmi[l...`, une ligne du navigateur
# melangee octet contre octet au prefixe d'une ligne du noyau.
#
# BO_SMP : coeurs (4 par defaut). BO_QEMU_KVM=1 : sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_serie_entrelacement.sh BOOTIMAGE [ECRIVAINS LIGNES PROCESSUS CHARGE]}
ECRIVAINS=${2:-3}
LIGNES=${3:-200}
PROCESSUS=${4:-120}
CHARGE=${5:-160}
SMP=${BO_SMP:-4}
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi

SCENARIO=scenario-serie-entrelacement
IMAGE=serie-entrelacement.img
LOG=serie-entrelacement.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG" tools/userland/out-serie-entrelacement
(cd tools/userland && OUT=out-serie-entrelacement ./build.sh musl >/dev/null)
mkdir -p "$SCENARIO/bin"
cp tools/userland/out-serie-entrelacement/serie-entrelacement-probe "$SCENARIO/bin/"
printf '/bin/serie-entrelacement-probe %s %s %s %s\necho SERIE_ENTRELACEMENT_AUTORUN_FIN\n' \
  "$ECRIVAINS" "$LIGNES" "$PROCESSUS" "$CHARGE" > "$SCENARIO/autorun"
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

echo "SERIE_ENTRELACEMENT smp=${SMP} accel=${ACCEL} ecrivains=${ECRIVAINS} lignes=${LIGNES} processus=${PROCESSUS} charge=${CHARGE}"
: > "$LOG"
# shellcheck disable=SC2086
qemu-system-x86_64 $ACCEL \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 2048 -smp "$SMP" -display none -no-reboot \
  -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 600))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE atteinte" >&2; break; fi
  grep -aqE 'KERNEL PANIC|panicked at' "$LOG" && { sleep 2; break; }
  grep -aq 'BOUCHAUD_SERIE_BILAN' "$LOG" && { sleep 2; break; }
  sleep 1
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

echecs=0
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs=$((echecs + 1)); fi; }
exige "aucune panique noyau" bash -c "! grep -aqE 'KERNEL PANIC|panicked at' '$LOG'"
exige "la sonde a fini, tous ses ecrivains a zero" grep -aq "SERIE_ENTRELACEMENT_FIN ecrivains=$ECRIVAINS lignes=$LIGNES processus=$PROCESSUS ecrivains_en_echec=0" "$LOG"
exige "le noyau a ecrit pendant ce temps (fautes voulues)" grep -aq 'PROCESS_FAULT' "$LOG"
exige "toutes les lignes intactes" python3 tools/ci/analyse_serie_entrelacement.py "$LOG" "$ECRIVAINS" "$LIGNES" "$CHARGE"
exige "aucune ligne emise sans le jeton ni en reentrance" grep -aqE 'BOUCHAUD_SERIE_BILAN emissions_a_la_borne=0 emissions_reentrees=0' "$LOG"
if [ "$echecs" -eq 0 ]; then
  echo "SERIE_ENTRELACEMENT_BANC_OK"
else
  echo "SERIE_ENTRELACEMENT_BANC_ECHEC n=$echecs"
  exit 1
fi
