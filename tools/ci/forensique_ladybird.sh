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
  | grep -aE '\[(SMP-SNAPSHOT|SMP-SPIN|SMP-TASK|SMP-LOAD|SMP-IPI|SCHED-RESUME|SCHED-FILE|SCHED-DUMP|SCHED-ORPHELINE|SCHED-NG-ATTENTE-VIVE|SCHED-NG-VEILLE|SCHED-NG-FILE|WAITQ-DETACHED|WAIT-WORD|FUTEX|PERF-PROC)\]' \
  | tail -n 120 | cut -c1-260 || true
# BOUCHAUD_SYSRQ_RELEVE_FILS_V1 : qui attend quoi, fil par fil -- le DERNIER
# releve demande par Alt+SysRq (tools/ci/releve_fils_qemu.sh), et la table
# pid -> processus Ladybird pour le lire.
echo "-- processus Ladybird crees / sortis (pid -> role) --"
sed -E 's/\x1b\[[0-9;]*m//g' "$JOURNAL" | tr -d '\r' \
  | grep -aoE '(\[LB\] PROCESS_(CREATE|EXIT) .*|PROCESS_EXIT t=[0-9]+ pid=[0-9]+ ppid=[0-9]+ image=[^ ]+ code=-?[0-9]+)' \
  | tail -n 40 | cut -c1-200 || true
echo "-- dernier releve des fils (Alt+SysRq) --"
sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$JOURNAL" | tr -d '\r' \
  | awk '/\[FIL-RELEVE\] debut/ { n = 0; delete l } /\[FIL-RELEVE\] debut/, /\[FIL-RELEVE\] fin/ { if ($0 !~ /\[SCHED-TACHE\]/) l[++n] = $0 }
         END { if (n == 0) print "(aucun releve publie)"; for (i = 1; i <= n && i <= 500; i++) print l[i] }' \
  | cut -c1-420 || true
# BOUCHAUD_SOUFFRANCE_SITE_DETENTEUR_V1 : quand tous les coeurs sont figes,
# `[DIAG-EN-SOUFFRANCE]` est la SEULE ligne qui sort, et ses champs `spin=` et
# `ipi=` depassent 260 caracteres : les trois dernieres, ENTIERES (gel de
# main@eb6f115 : la ligne coupee a fait perdre l'etat des coeurs 2 et 3).
echo "-- noyau fige : dernieres lignes [DIAG-EN-SOUFFRANCE], entieres --"
sed -E 's/\x1b\[[0-9;]*m//g' "$JOURNAL" | tr -d '\r' | grep -aF '[DIAG-EN-SOUFFRANCE]' | tail -n 3 || true
echo "-- 60 dernieres lignes du journal, hors releves memoire periodiques et releve des fils --"
sed -E 's/\x1b\[[0-9;]*m//g' "$JOURNAL" | tr -d '\r' | grep -avE 'MM-NG6|\[FIL-FORENSIQUE\]|\[SCHED-TACHE\]' | tail -n 60 | cut -c1-260 || true
echo "== FORENSIQUE FIN =="
exit 0
