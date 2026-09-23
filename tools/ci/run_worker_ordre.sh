#!/usr/bin/env bash
# LA PREMIERE POSITION, OU L'ORIGINE ?
#
# BOUCHAUD_C57_LA_PREMIERE_POSITION_OU_L_ORIGINE
#
# Le dossier porte deux observations, prises sur deux runs differents -- donc
# deux binaires et deux coureurs :
#
#     ordre http,blob   http_1 lent (~148 s)   blob_2 immediat
#     ordre blob,http   blob_1 lent (122 s)    http_2 9 s
#
# La lenteur suit la PREMIERE POSITION dans les deux cas. C'est fort, mais ce
# n'est pas une experience : rien n'etait tenu constant.
#
# Ce banc tient tout constant sauf l'ordre : MEME binaire, MEME noyau, MEME
# coureur, deux demarrages QEMU FROIDS -- donc deux caches de pages vierges.
#
#   si le PREMIER est lent des deux cotes   -> demarrage a froid du WebWorker
#   si `blob` reste lent en second          -> l'origine est en cause
#   si l'ordre change autre chose           -> on rapporte, on n'explique pas
#
# Il ne remplace PAS le smoke de reference : celui-ci garde son ordre et ses
# noms de fichiers.
set -euo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:?usage: run_worker_ordre.sh BOOTIMAGE NATIVE_DIR}
OUT=${2:?usage: run_worker_ordre.sh BOOTIMAGE NATIVE_DIR}

echecs=0

# Chaque bras est un demarrage COMPLET et SEPARE. Reutiliser la machine
# donnerait au second bras un cache deja chaud, ce qui detruirait la mesure.
for ordre in blob http; do
    echo
    echo "=== bras ordre=$ordre (demarrage froid) ==="
    BO_SMOKE_SUFFIXE="-ordre-$ordre" BO_SMOKE_ORDRE="$ordre" \
        tools/ci/run_ladybird_browser_host.sh "$BOOT" "$OUT" \
        > "ordre-$ordre.sortie" 2>&1 || true
    if ! grep -q "LADYBIRD_BROWSER_HOST_OK" "ordre-$ordre.sortie"; then
        echo "ordre : le bras $ordre n'est pas alle au bout" >&2
        tail -15 "ordre-$ordre.sortie" >&2
        echecs=$((echecs + 1))
    fi
done

echo
echo "== resultats par rang, les deux bras =="
python3 tools/ci/analyse_worker_ordre.py \
    "ordre-blob.sortie" "ordre-http.sortie" || echecs=$((echecs + 1))

if [ "$echecs" -ne 0 ]; then
    echo "ORDRE_WORKER_INCONCLUSIF echecs=$echecs" >&2
    exit 1
fi
echo "ORDRE_WORKER_OK"
