#!/usr/bin/env bash
# A/B local : services separes (A) contre BouchaudServices en liens physiques (B).
# Meme bootimage, meme artefact, meme QEMU (TCG), paires alternees A,B / B,A.
#   tools/ci/run_ladybird_services_ab.sh BOOT NATIF SORTIE [PAIRES]
# Mesures : python3 tools/ci/mesures_services_ab.py SORTIE/*-[AB]
set -uo pipefail
cd "$(dirname "$0")/../.."
boot=$1 natif=$2 sortie=$3 paires=${4:-3}
mkdir -p "$sortie"
{ sha256sum "$boot" "$natif"/BouchaudBrowserHost "$natif"/WebContent "$natif"/BouchaudServices 2>/dev/null
  git rev-parse HEAD; qemu-system-x86_64 --version | head -1; nproc; } > "$sortie/identite.txt"
printf 'tour\tbras\tcode\tetat\tsecondes\n' > "$sortie/verdicts.tsv"
for ((t = 1; t <= paires; t++)); do
    if ((t % 2)); then bras=(A B); else bras=(B A); fi
    for b in "${bras[@]}"; do
        d=$(printf '%s/%02d-%s' "$sortie" "$t" "$b"); mkdir -p "$d"
        cp -- "$boot" "$d/amorce.bin"
        suf="-ab-$t-$b"; unique=0; [[ $b == B ]] && unique=1
        debut=$(date +%s)
        BO_SMOKE_SERVICES_UNIQUES=$unique BO_SMOKE_SUFFIXE="$suf" \
            tools/ci/run_ladybird_browser_host.sh "$d/amorce.bin" "$natif" > "$d/smoke.log" 2>&1
        code=$?
        duree=$(( $(date +%s) - debut ))
        rm -f -- "$d/amorce.bin" "ladybird-browser-host${suf}.img"
        for j in serie-browser-host${suf}.log fixture-browser-host${suf}.log surface-browser-host${suf}.ppm surface-browser-host${suf}-t1.ppm surface-tentatives${suf}.txt convergence-browser-host${suf}.json; do
            [[ -f $j ]] && mv -- "$j" "$d/"
        done
        etat=echec; grep -qFx 'LADYBIRD_FUNCTIONAL_SMOKE ok' "$d/smoke.log" && etat=ok
        printf '%s\t%s\t%s\t%s\t%s\n' "$t" "$b" "$code" "$etat" "$duree" >> "$sortie/verdicts.tsv"
    done
done
cat "$sortie/verdicts.tsv"
python3 tools/ci/mesures_services_ab.py "$sortie"/[0-9]*-[AB] > "$sortie/mesures.jsonl"
cat "$sortie/mesures.jsonl"
