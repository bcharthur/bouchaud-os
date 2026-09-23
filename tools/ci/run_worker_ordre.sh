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
    # La VM appartient a l'hote, et la boucle attend le verdict de la PAGE.
    # Sans ces deux modes, la fin de l'autorun tuait l'experience avant qu'elle
    # ne commence -- run 35907201865.
    BO_SMOKE_SUFFIXE="-ordre-$ordre" BO_SMOKE_ORDRE="$ordre" \
    BO_SMOKE_KEEP_GUEST_ALIVE=1 BO_SMOKE_ATTEND_AB=1 \
        tools/ci/run_ladybird_browser_host.sh "$BOOT" "$OUT" \
        > "ordre-$ordre.sortie" 2>&1 || true
    if ! grep -q "LADYBIRD_BROWSER_HOST_OK" "ordre-$ordre.sortie"; then
        echo "ordre : le bras $ordre n'est pas alle au bout" >&2
        tail -15 "ordre-$ordre.sortie" >&2
        echecs=$((echecs + 1))
    fi

    # UN BANC DOIT PROUVER LA CONFIGURATION QU'IL CROIT TESTER.
    #
    # BOUCHAUD_C58_UN_HEREDOC_PROTEGE_N_EXPANSE_RIEN
    #
    # Au run 35900151523, les deux bras ont charge la MEME page : l'URL
    # n'emportait pas le parametre. Les deux boots ont pourtant reussi, et
    # rien ne l'a dit -- il a fallu lire l'analyseur se plaindre de n'avoir
    # qu'un bras pour decouvrir que l'experience n'avait jamais eu lieu.
    #
    # Ces deux verifications ferment ce trou des la fin du bras : l'URL
    # demandee, et l'ordre que la page dit avoir applique.
    if ! grep -aq "BO_SMOKE_URL ordre=$ordre " "ordre-$ordre.sortie"; then
        echo "ordre : bras $ordre -- l'URL demandee n'a pas ete publiee" >&2
        grep -a "BO_SMOKE_URL" "ordre-$ordre.sortie" >&2 || true
        echecs=$((echecs + 1))
    fi
    # UN BUREAU QUI REVIENT AVANT LE VERDICT EST UNE PANNE.
    #
    # Le filet de duree de vie garde la machine debout pour qu'on puisse
    # OBSERVER cela -- pas pour le taire. Si `desktop` est revenu, le
    # navigateur est mort et le bras ne vaut rien, meme si QEMU tourne encore.
    if grep -aq "AUTORUN_DESKTOP_RETURN" "ordre-$ordre.sortie" \
       && ! grep -aq "HOST_WORKER_AB_COMPLETE" "ordre-$ordre.sortie"; then
        echo "ordre : bras $ordre -- desktop est revenu AVANT le verdict" >&2
        grep -aE "AUTORUN_DESKTOP_RETURN|RUN_NOYAU_RETOUR|RUN_NOYAU_VIVANT|PROCESS_EXIT" \
            "ordre-$ordre.sortie" | tail -8 >&2 || true
        echecs=$((echecs + 1))
    fi
    if ! grep -aq "HOST_WORKER_ORDRE ordre=$ordre " "ordre-$ordre.sortie"; then
        echo "ordre : demande=$ordre mais l'ordre reel n'a pas ete observe" >&2
        grep -a "HOST_WORKER_ORDRE" "ordre-$ordre.sortie" >&2 || true
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
