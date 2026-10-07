#!/usr/bin/env bash
# Imprime le contexte d'une panique noyau d'un journal serie, s'il y en a une
# (BOUCHAUD_EXTRAIT_PANIQUE_V1).
#
#   tools/ci/extrait_panique.sh JOURNAL_SERIE [DOSSIER_BINAIRES]
#
# Avec DOSSIER_BINAIRES (l'artefact Ladybird du run), il symbolise aussi les
# fautes des processus (tools/ci/symbolise_fautes.py,
# BOUCHAUD_SYMBOLISE_FAUTES_V1).
#
# Les bancs l'appellent EN DERNIER : l'API des journaux de job ne rend que la
# fin, et l'artefact du journal serie n'est pas lisible partout. Au run
# 37584587000, l'endurance a fini sur « panique » sans que le texte de la
# panique ne soit imprime nulle part. Rend 0 dans tous les cas : ce n'est pas
# lui qui juge, il montre.
set -uo pipefail
JOURNAL=${1:?usage: extrait_panique.sh JOURNAL_SERIE [DOSSIER_BINAIRES]}
BINAIRES=${2:-}
[ -f "$JOURNAL" ] || exit 0
if [ -n "$BINAIRES" ] && [ -d "$BINAIRES" ]; then
  python3 "$(dirname "$0")/symbolise_fautes.py" "$JOURNAL" "$BINAIRES" || true
fi
propre=$(sed -E 's/\x1b\[[0-9;]*m//g' "$JOURNAL" | tr -d '\r')
ligne=$(printf '%s\n' "$propre" | grep -anE 'KERNEL PANIC|panicked at|PANIC_' | head -1 | cut -d: -f1)
[ -n "$ligne" ] || exit 0
echo
echo "== panique noyau (journal $(basename "$JOURNAL"), ligne $ligne) =="
debut=$(( ligne > 25 ? ligne - 25 : 1 ))
printf '%s\n' "$propre" | sed -n "${debut},$(( ligne + 60 ))p" | cut -c1-240 | sed 's/^/  /'
exit 0
