#!/usr/bin/env bash
# UNE TACHE PRETE N'EST JAMAIS PERDUE PAR L'ELECTION (P13_ENTREE_VALIDE_JAMAIS_JETEE_V1).
#
#   tools/ci/run_rejet_election.sh BOOTIMAGE_A_OPTION [remise|sans-remise] [N]
#
# Image construite avec `--features banc-rejet-election`. `ordo-rejet-injecte N`
# fait rejeter par l'election locale N entrees pourtant ELIGIBLES de taches
# utilisateur -- la condition un instant fausse du C19 (banc console TCG SMP4
# du 28/10 : `[SCHED-ORPHELINE] tid=117 ... jetees_non_eligibles=1`, machine
# figee). Pendant ce temps, `serie-entrelacement-probe` fait naitre et mourir
# des processus sur tous les coeurs.
#
#   remise      (correctif) : chaque entree rejetee est remise ; la sonde finit,
#               aucun `[SCHED-ORPHELINE]`, remises >= injectes > 0.
#   sans-remise (temoin)    : la perte d'avant le correctif ; un
#               `[SCHED-ORPHELINE]` DOIT etre signale -- sinon le banc ne prouve
#               plus rien.
#
# BO_SMP : coeurs (4 par defaut). BO_QEMU_KVM=1 : sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_rejet_election.sh BOOTIMAGE_A_OPTION [remise|sans-remise] [N]}
BRAS=${2:-remise}
N=${3:-50}
SMP=${BO_SMP:-4}
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
SORTIE=${REJET_SORTIE:-$PWD/rejet-election-$BRAS}
rm -rf "$SORTIE"; mkdir -p "$SORTIE/scenario/bin"
(cd tools/userland && OUT=out-rejet-election ./build.sh musl >/dev/null)
cp tools/userland/out-rejet-election/serie-entrelacement-probe "$SORTIE/scenario/bin/"
{
  if [ "$BRAS" = sans-remise ]; then echo "ordo-rejet-injecte $N sans-remise"; else echo "ordo-rejet-injecte $N"; fi
  echo '/bin/serie-entrelacement-probe 3 200 120 160 0'
  echo 'ordo-rejet-injecte etat'
  echo 'echo REJET_ELECTION_FIN'
} > "$SORTIE/scenario/autorun"
(cd tools/userland && IMAGE="$SORTIE/disque.img" ./mkdisk.sh "$SORTIE/scenario" >/dev/null)
cp "$BOOT" "$SORTIE/boot.bin"
LOG="$SORTIE/serie.log"
: > "$LOG"
echo "REJET_ELECTION smp=$SMP accel=$ACCEL bras=$BRAS rejets=$N"
# shellcheck disable=SC2086
qemu-system-x86_64 $ACCEL -drive format=raw,file="$SORTIE/boot.bin" -drive format=raw,file="$SORTIE/disque.img" \
  -m 2048 -smp "$SMP" -display none -no-reboot -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 300))
while kill -0 "$PID" 2>/dev/null && (( SECONDS < LIMITE )); do
  grep -aqE 'KERNEL PANIC|panicked at' "$LOG" && break
  grep -aq 'REJET_ELECTION_FIN' "$LOG" && { sleep 1; break; }
  grep -aq '\[SCHED-ORPHELINE\] total=' "$LOG" && { sleep 1; break; }
  sleep 1
done
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
propre() { sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | tr -d '\r'; }
ETAT=$(propre | grep -aoE 'ORDO_REJET_ETAT injectes=[0-9]+ remises=[0-9]+' | tail -1 || true)
INJECTES=$(echo "$ETAT" | grep -oE 'injectes=[0-9]+' | cut -d= -f2)
REMISES=$(echo "$ETAT" | grep -oE 'remises=[0-9]+' | cut -d= -f2)
echo "  etat    ${ETAT:-aucun releve ORDO_REJET_ETAT}"
propre | grep -aE '\[SCHED-ORPHELINE\]' | head -3 | sed 's/^.*\[SCHED-ORPHELINE\]/  orpheline :/' || true

echecs=0
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs=$((echecs + 1)); fi; }
exige "aucune panique noyau" bash -c "! grep -aqE 'KERNEL PANIC|panicked at' '$LOG'"
exige "l'injection a ete armee" grep -aq "ORDO_REJET_INJECTE n=$N" "$LOG"
if [ "$BRAS" = sans-remise ]; then
  exige "temoin : une tache prete perdue est signalee ([SCHED-ORPHELINE])" grep -aq '\[SCHED-ORPHELINE\] total=' "$LOG"
else
  exige "la sonde a fini, tous ses ecrivains a zero" grep -aq "SERIE_ENTRELACEMENT_FIN ecrivains=3 lignes=200 processus=120 ecrivains_en_echec=0" "$LOG"
  exige "au moins un rejet injecte" test "${INJECTES:-0}" -gt 0
  exige "chaque rejet injecte remis (remises ${REMISES:-?} >= injectes ${INJECTES:-?})" test "${REMISES:-0}" -ge "${INJECTES:-1}"
  exige "aucune tache prete perdue" bash -c "! grep -aq '\[SCHED-ORPHELINE\]' '$LOG'"
  exige "l'autorun est alle au bout" grep -aq 'REJET_ELECTION_FIN' "$LOG"
fi
if [ "$echecs" -eq 0 ]; then
  echo "REJET_ELECTION_BANC_OK bras=$BRAS"
else
  echo "REJET_ELECTION_BANC_ECHEC bras=$BRAS n=$echecs"
  exit 1
fi
