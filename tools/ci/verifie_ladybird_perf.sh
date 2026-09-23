#!/usr/bin/env bash
# Le verdict de PERFORMANCE de Ladybird, separe du fonctionnel.
#
# BOUCHAUD_C44_DEUX_STATUTS_REELS
#
# `BOUCHAUD_C32_DEUX_VERDICTS` avait separe les deux verdicts dans la SORTIE du
# banc, mais un seul travail de CI les portait tous les deux -- et il ne
# bloquait que sur le fonctionnel. `BO_SMOKE_PERF_BLOQUANT` n'etait defini par
# aucun workflow : une regression de performance laissait donc la CI verte.
#
# Ce script est le verdict de performance, et il est TOUJOURS fail-closed. Il
# ne relance pas QEMU : il relit le journal deja produit, pour qu'un seul
# demarrage serve aux deux analyses.
#
# Regle qui ne doit pas bouger : « fonctionnel mais lent » ne devient jamais
# « fonctionnel casse », et l'inverse non plus. Les deux statuts sont
# independants parce que les deux decisions le sont.
set -euo pipefail

LOG=${1:-serie-browser-host.log}

if [ ! -f "$LOG" ]; then
    echo "performance : journal absent : $LOG" >&2
    echo "              rien n'a pu etre mesure -- ce n'est pas un succes" >&2
    echo "LADYBIRD_PERFORMANCE_SMOKE fail raison=journal_absent"
    exit 1
fi

echo "== budgets de performance =="
echecs=0
lignes=0
while IFS= read -r ligne; do
    lignes=$((lignes + 1))
    printf '  %s\n' "$ligne"
    case "$ligne" in
        *' FAIL '*) echecs=$((echecs + 1)) ;;
    esac
done < <(grep -aoE 'HOST_WORKER_[A-Z_]*PERF[A-Z_]* (OK|FAIL).*' "$LOG" | sed 's/\r//g' || true)

if [ "$lignes" -eq 0 ]; then
    # INCONCLUSIF N'EST PAS PASS. Le banc a tourne sans produire une seule
    # mesure : soit la page n'est jamais arrivee jusqu'aux workers, soit les
    # lignes ont change de nom. Les deux demandent un regard.
    echo "LADYBIRD_PERFORMANCE_SMOKE inconclusif raison=aucune_ligne_perf"
    echo "performance : aucune ligne _PERF dans le journal -- rien n'a ete mesure" >&2
    exit 1
fi

if [ "$echecs" -ne 0 ]; then
    echo "LADYBIRD_PERFORMANCE_SMOKE fail hors_budget=$echecs/$lignes"
    echo "performance : $echecs mesure(s) sur $lignes hors budget" >&2
    echo "              le navigateur FONCTIONNE ; il est trop lent." >&2
    exit 1
fi

echo "LADYBIRD_PERFORMANCE_SMOKE ok mesures=$lignes"
