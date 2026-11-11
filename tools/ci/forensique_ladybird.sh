#!/usr/bin/env bash
# BOUCHAUD_LADYBIRD_FORENSIQUE_V1 (#283) -- etat du noyau pendant un echec.
#
#   tools/ci/forensique_ladybird.sh JOURNAL_SERIE [ETIQUETTE]
#
# Un demarrage de Ladybird fige (~1 sur 50) ne laissait dans le job que la fin
# de sortie du banc ; l'etat de l'ordonnanceur que le noyau ecrit toutes les
# 30 s PENDANT le gel restait dans un artefact. Ce script le recopie dans le
# job : taches pretes/bloquees/sur coeur, tache et appel systeme courants par
# coeur, verrous tenus, futex, puis les 60 dernieres lignes hors releves
# memoire. Lecture seule ; ne decide d'aucun verdict ; rend toujours 0.
set -uo pipefail
JOURNAL=${1:-}
ETIQUETTE=${2:-}
if [ -z "$JOURNAL" ] || [ ! -f "$JOURNAL" ]; then
  echo "== FORENSIQUE : journal serie absent (${JOURNAL:-?}) =="
  exit 0
fi
echo "== FORENSIQUE DEBUT ($ETIQUETTE) =="
sed -E 's/\x1b\[[0-9;]*m//g' "$JOURNAL" | tr -d '\r' \
  | grep -aE '\[(SMP-SNAPSHOT|SMP-SPIN|SMP-TASK|SMP-LOAD|SMP-IPI|SCHED-RESUME|SCHED-TACHE|SCHED-FILE|SCHED-DUMP|SCHED-ORPHELINE|SCHED-NG-ATTENTE-VIVE|SCHED-NG-VEILLE|SCHED-NG-FILE|WAITQ-DETACHED|WAIT-WORD|FUTEX|PERF-PROC)\]' \
  | tail -n 120 | cut -c1-260 || true
echo "-- 60 dernieres lignes du journal, hors releves memoire periodiques --"
sed -E 's/\x1b\[[0-9;]*m//g' "$JOURNAL" | tr -d '\r' | grep -av 'MM-NG6' | tail -n 60 | cut -c1-260 || true
echo "== FORENSIQUE FIN =="
exit 0
