#!/usr/bin/env bash
# AVANT / APRES DE LIAISON, DANS LE MEME RUN.
#
# BOUCHAUD_C65_STATIC_PIE_OU_ET_EXEC
#
# Deux demarrages froids identiques, seul le WebWorker change :
#
#     A  static-PIE   (reference)   ~405 000 relocations avant main
#     B  static EXEC  (variante)    aucune relocation de demarrage
#
# La variance entre coureurs atteint un facteur deux sur le MEME SHA. Les deux
# bras tournent donc sur la MEME machine, dans le MEME run, a quelques minutes
# d'intervalle -- c'est la seule facon de rendre l'ecart attribuable.
#
# Ce banc ne remplace ni le smoke de reference ni le budget : il tranche une
# question de cause, et son resultat decide du correctif de production.
set -euo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:?usage: run_variante_elf.sh BOOTIMAGE NATIVE_DIR}
OUT=${2:?usage: run_variante_elf.sh BOOTIMAGE NATIVE_DIR}

if [ ! -f "$OUT/WebWorker.etexec" ]; then
    echo "variante : $OUT/WebWorker.etexec absent -- rien a comparer" >&2
    echo "           l'etape « Variante ET_EXEC » a-t-elle reussi ?" >&2
    exit 1
fi

echo "== forme des deux ELF =="
for f in WebWorker WebWorker.etexec; do
    printf '  %-20s ' "$f"
    printf 'type=%s bytes=%s relasz=%s relacount=%s\n' \
        "$(readelf -h "$OUT/$f" | awk '/Type:/ {print $2}')" \
        "$(stat -c %s "$OUT/$f")" \
        "$(readelf -d "$OUT/$f" 2>/dev/null | awk '/RELASZ/ {gsub(/[^0-9]/,"",$3); print $3}' | head -1)" \
        "$(readelf -d "$OUT/$f" 2>/dev/null | awk '/RELACOUNT/ {print $3}' | head -1)"
done

# Le binaire de reference est mis de cote UNE fois : chaque bras remet la
# bonne version en place avant son demarrage.
cp "$OUT/WebWorker" "$OUT/WebWorker.pie"

echecs=0
for bras in pie etexec; do
    echo
    echo "=== bras $bras (demarrage froid) ==="
    cp "$OUT/WebWorker.$bras" "$OUT/WebWorker"
    printf '  WebWorker en place : type=%s bytes=%s\n' \
        "$(readelf -h "$OUT/WebWorker" | awk '/Type:/ {print $2}')" \
        "$(stat -c %s "$OUT/WebWorker")"

    BO_SMOKE_SUFFIXE="-elf-$bras" BO_SMOKE_ORDRE=blob \
    BO_SMOKE_KEEP_GUEST_ALIVE=1 BO_SMOKE_ATTEND_AB=1 \
        tools/ci/run_ladybird_browser_host.sh "$BOOT" "$OUT" \
        > "elf-$bras.sortie" 2>&1 || true

    if ! grep -aq "HOST_WORKER_AB_COMPLETE" "elf-$bras.sortie"; then
        echo "variante : bras $bras sans verdict terminal" >&2
        grep -aE "AUTORUN_DESKTOP_RETURN|RUN_NOYAU_RETOUR|PROCESS_EXIT" \
            "elf-$bras.sortie" | tail -6 >&2 || true
        echecs=$((echecs + 1))
    fi
done

# Le binaire de reference est remis en place : l'artefact ne doit pas rester
# sur la variante apres le banc.
cp "$OUT/WebWorker.pie" "$OUT/WebWorker"

echo
echo "== avant / apres =="
python3 tools/ci/analyse_variante_elf.py "elf-pie.sortie" "elf-etexec.sortie" \
    || echecs=$((echecs + 1))

if [ "$echecs" -ne 0 ]; then
    echo "VARIANTE_ELF_INCONCLUSIF echecs=$echecs" >&2
    exit 1
fi
echo "VARIANTE_ELF_OK"
