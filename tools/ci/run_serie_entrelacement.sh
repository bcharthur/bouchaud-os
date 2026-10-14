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
#
# BOUCHAUD_SERIE_JAMAIS_SANS_JETON_V1 -- BANC_SUSPENSION_MS=N : un fil noyau
# prend le jeton 1,5 s apres le depart et le garde N ms interruptions masquees,
# au milieu d'une ligne (un processeur virtuel retire par l'hote). Exige : sa
# ligne entiere, aucune ligne sans jeton, des lignes differees puis videes.
# Au-dela de ce que les anneaux tiennent (BANC_PERDUES_ADMISES=1), une ligne
# peut manquer -- comptee perdue par le noyau --, jamais etre coupee.
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
SUSPENSION=${BANC_SUSPENSION_MS:-0}
{
  if [ "$SUSPENSION" -gt 0 ]; then echo "serie-detenteur-suspendu 1500 $SUSPENSION"; fi
  printf '/bin/serie-entrelacement-probe %s %s %s %s\necho SERIE_ENTRELACEMENT_AUTORUN_FIN\n' \
    "$ECRIVAINS" "$LIGNES" "$PROCESSUS" "$CHARGE"
} > "$SCENARIO/autorun"
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

echo "SERIE_ENTRELACEMENT smp=${SMP} accel=${ACCEL} ecrivains=${ECRIVAINS} lignes=${LIGNES} processus=${PROCESSUS} charge=${CHARGE} suspension_ms=${SUSPENSION}"
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
ADMISES=""
[ "${BANC_PERDUES_ADMISES:-0}" = 1 ] && ADMISES=--perdues-admises
exige "toutes les lignes intactes" python3 tools/ci/analyse_serie_entrelacement.py "$LOG" "$ECRIVAINS" "$LIGNES" "$CHARGE" $ADMISES
exige "aucune ligne emise sans le jeton ni en reentrance" grep -aqE 'BOUCHAUD_SERIE_BILAN emissions_a_la_borne=0 emissions_reentrees=0' "$LOG"
# Le bilan lui-meme, pour le journal du job.
sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'BOUCHAUD_SERIE_BILAN' | tail -1 | sed 's/^.*BOUCHAUD_SERIE_BILAN/  bilan :/'
if [ "${BANC_PERDUES_ADMISES:-0}" = 1 ]; then
  exige "chaque ligne differee est videe ou comptee perdue" bash -c "sed -E 's/\x1b\[[0-9;]*m//g' '$LOG' | grep -aqE 'lignes_differees=([0-9]+) lignes_perdues=[0-9]+ lignes_vidangees=\1'"
else
  exige "aucune ligne differee perdue, toutes videes avant le bilan" bash -c "sed -E 's/\x1b\[[0-9;]*m//g' '$LOG' | grep -aqE 'lignes_differees=([0-9]+) lignes_perdues=0 lignes_vidangees=\1'"
fi
if [ "$SUSPENSION" -gt 0 ]; then
  exige "le detenteur suspendu a tenu le jeton ${SUSPENSION} ms" grep -aq "SERIE_DETENTEUR_SUSPENDU_FAIT pris=1 duree_ms=${SUSPENSION}" "$LOG"
  exige "sa ligne est sortie entiere" bash -c "sed -E 's/\x1b\[[0-9;]*m//g' '$LOG' | tr -d '\r' | grep -aqE '\] SERIE_DETENTEUR_SUSPENDU debut et fin, tenu ${SUSPENSION} ms\$'"
  exige "la suspension a force des lignes differees" bash -c "sed -E 's/\x1b\[[0-9;]*m//g' '$LOG' | grep -aqE 'lignes_differees=[1-9]'"
fi
if [ "$echecs" -eq 0 ]; then
  echo "SERIE_ENTRELACEMENT_BANC_OK"
else
  echo "SERIE_ENTRELACEMENT_BANC_ECHEC n=$echecs"
  exit 1
fi
