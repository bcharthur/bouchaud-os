#!/usr/bin/env bash
#
# BOUCHAUD_FENETRE_MIGRATION_V1 (#283) : aucune marque par coeur
# (`preempt_count`, `verrous_simples`) ne doit etre posee sur un autre coeur
# que celui qui execute la tache.
#
#   tools/ci/run_fenetre_migration.sh BOOTIMAGE_TEMOIN [SMP]
#
# BOOTIMAGE_TEMOIN : noyau compile avec `banc-fenetre-migration` (jamais
# livre). Quatre fils noyau exercent `preempt::disable/enable` et
# `SpinLock::lock` pendant 30 s, sous demandes de preemption ciblees vers tous
# les coeurs, la fenetre lecture-du-coeur/pose-de-la-marque elargie pour eux.
# Exige :
#   1. la ligne `BOUCHAUD_BANC_FENETRE fin` avec fils_finis=4/4 ;
#   2. des MIGRATIONS observees et des preemptions noyau accordees (sans
#      elles, le banc n'aurait rien exerce) ;
#   3. fenetre_preempt=0 et fenetre_verrou=0 ;
#   4. aucune panique (en particulier `preempt_count underflow`).
# Sur le noyau d'avant BOUCHAUD_FENETRE_MIGRATION_V1, le meme temoin donne des
# marques deplacees ou la panique : c'est le temoin negatif, joue a la main
# (voir la PR).
set -uo pipefail
cd "$(dirname "$0")/../.."
TEMOIN=${1:?usage: run_fenetre_migration.sh BOOTIMAGE_TEMOIN [SMP]}
SMP=${2:-4}
OUT=${OUT:-fenetre-migration}
mkdir -p "$OUT"
LOG="$OUT/serie-smp${SMP}.log"
: > "$LOG"
qemu-system-x86_64 -cpu max \
  -drive format=raw,file="$TEMOIN",snapshot=on \
  -m 2048 -smp "$SMP" -display none -no-reboot \
  -serial file:"$LOG" >/dev/null 2>&1 &
PID=$!
LIMITE=$((SECONDS + 900))
etat=echeance
while kill -0 "$PID" 2>/dev/null && (( SECONDS < LIMITE )); do
  if grep -aqE 'KERNEL PANIC|panicked at' "$LOG"; then etat=panique; sleep 2; break; fi
  if grep -aqF 'BOUCHAUD_BANC_FENETRE fin' "$LOG"; then etat=fini; break; fi
  if grep -aqF 'BOUCHAUD_BANC_FENETRE_ECHEC' "$LOG"; then etat=armement; break; fi
  sleep 2
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
texte=$(sed -E 's/\x1b\[[0-9;]*m//g' "$LOG")

echecs=0
echec() { echo "  ECHEC   $1"; echecs=$((echecs + 1)); }
ok() { echo "  ok      $1"; }
echo "FENETRE_MIGRATION smp=${SMP} etat=${etat}"
awk '/BOUCHAUD_BANC_FENETRE armement/ { vu = 1 } END { exit !vu }' <<<"$texte" \
  && ok "temoin arme" || echec "le temoin n'a pas ete arme"
case "$etat" in
  fini) ok "le banc est alle au bout" ;;
  panique) echec "panique noyau"
           awk '/panicked at|KERNEL PANIC/ && n < 4 { print "          " substr($0, 1, 200); n++ }' <<<"$texte" ;;
  *) echec "banc inacheve (etat=${etat})" ;;
esac
ligne=$(awk '/BOUCHAUD_BANC_FENETRE fin/ { l = $0 } END { print l }' <<<"$texte")
if [ -n "$ligne" ]; then
  echo "  ${ligne#*BOUCHAUD_BANC_FENETRE }"
  v() { sed -nE "s/.* $1=([0-9]+).*/\\1/p" <<<"$ligne"; }
  [ "$(sed -nE 's/.* fils_finis=([0-9]+)\/([0-9]+).*/\1 \2/p' <<<"$ligne")" = "4 4" ] \
    && ok "quatre fils au bout" || echec "fils inacheves"
  (( $(v migrations) > 0 )) && ok "migrations observees : $(v migrations)" \
    || echec "aucune migration : le banc n'a rien exerce"
  (( $(v preemptions_noyau) > 0 )) && ok "preemptions noyau accordees : $(v preemptions_noyau)" \
    || echec "aucune preemption noyau : le banc n'a rien exerce"
  (( $(v pauses) > 0 )) && ok "fenetres elargies : $(v pauses)" || echec "aucune fenetre elargie"
  [ "$(v fenetre_preempt)" = 0 ] && ok "fenetre_preempt=0" \
    || echec "preempt_count marque sur un autre coeur : $(v fenetre_preempt)"
  [ "$(v fenetre_verrou)" = 0 ] && ok "fenetre_verrou=0" \
    || echec "verrous_simples marque sur un autre coeur : $(v fenetre_verrou)"
else
  echec "aucune ligne BOUCHAUD_BANC_FENETRE fin"
fi
if (( echecs > 0 )); then
  echo "FENETRE_MIGRATION_ECHEC smp=${SMP} echecs=${echecs}"
  exit 1
fi
echo "FENETRE_MIGRATION_OK smp=${SMP}"
