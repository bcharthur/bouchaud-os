#!/usr/bin/env bash
# Même Ladybird dynamique et même transport Venus, Skia CPU puis Vulkan.
set -euo pipefail
cd "$(dirname "$0")/../.."
if [[ $# != 5 || ! $5 =~ ^([3-9]|1[0-2])$ || -z ${DISPLAY:-} ]]; then
    echo 'usage: DISPLAY=:N run_ladybird_skia_ab.sh BOOTIMAGE NATIVE_DIR ROOTFS_VENUS SORTIE REPS_3_A_12' >&2
    exit 2
fi
boot=$(realpath -e "$1")
natif=$(realpath -e "$2")
rootfs=$(realpath -e "$3")
sortie=$(realpath -m "$4")
repetitions=$5
accel=${BO_SMOKE_ACCEL:-tcg}
timings=${BO_SMOKE_SKIA_TIMINGS:-0}
compare_readback=${BO_SMOKE_SKIA_COMPARE_READBACK:-0}
compare_submit=${BO_SMOKE_SKIA_COMPARE_SUBMIT:-0}
if [[ $compare_readback == 1 && $compare_submit == 1 ]]; then
    echo 'Sélectionner une seule comparaison Vulkan.' >&2
    exit 2
fi
bras_pair=(cpu vulkan)
if [[ $compare_readback == 1 ]]; then
    timings=1
    bras_pair=(vulkan-full vulkan-partial)
elif [[ $compare_submit == 1 ]]; then
    timings=1
    bras_pair=(vulkan-explicit-submit vulkan-readback-submit)
fi
case "$accel" in
    tcg|kvm) ;;
    *) echo 'BO_SMOKE_ACCEL doit valoir tcg ou kvm.' >&2; exit 2 ;;
esac
qemu=$(command -v qemu-system-x86_64)
qemu_version=$("$qemu" --version)
[[ -f $boot && -d $natif && -d $rootfs ]]
mkdir -p target
exec 9>target/ladybird-skia-ab.lock
if ! flock -n 9; then
    echo 'Une campagne Skia utilise déjà le banc.' >&2
    exit 2
fi
mkdir -- "$sortie"

# Le smoke recrée sa scène et écoute sur un port fixe. Les bras passent
# un par un et l'image d'amorçage est copiée avant chaque démarrage.
identite() {
    sha256sum -- "$boot" "$qemu" tools/health/browser_host_fixture.py \
        tools/ci/run_ladybird_skia_ab.sh tools/ci/run_ladybird_browser_host.sh tools/ci/bench_ladybird_readback.py
    find -L "$natif" "$rootfs" -type f -print0 | LC_ALL=C sort -z \
        | xargs -0 -r sha256sum --zero | sha256sum
}
identite > "$sortie/entrees.sha256"
printf '%s\n' "$qemu_version" > "$sortie/qemu-version.txt"
printf '%s\n' "$accel" > "$sortie/acceleration.txt"
git rev-parse HEAD > "$sortie/source.txt"
printf '%s\n' "$timings" > "$sortie/skia-timings.txt"
printf 'tour\tbackend\tcode\tetat\n' > "$sortie/verdicts.tsv"

echec=0
for ((tour = 1; tour <= repetitions; tour++)); do
    if ((tour % 2)); then bras=("${bras_pair[@]}"); else bras=("${bras_pair[1]}" "${bras_pair[0]}"); fi
    for essai in "${bras[@]}"; do
        backend=${essai%%-*}
        partial=0
        if [[ $essai == vulkan-partial ]]; then partial=1; fi
        readback_submit=0
        if [[ $essai == vulkan-readback-submit ]]; then readback_submit=1; fi
        dossier=$(printf '%s/%02d-%s' "$sortie" "$tour" "$essai")
        mkdir -- "$dossier"
        cp --reflink=auto -- "$boot" "$dossier/amorce.bin"
        suffixe="-skia-ab-${BASHPID}-${tour}-${essai}"
        code=0
        BO_SMOKE_ACCEL="$accel" BO_SMOKE_AFFICHAGE=venus BO_SMOKE_SKIA_BACKEND="$backend" \
            BO_SMOKE_SKIA_TIMINGS="$timings" BO_SMOKE_SKIA_PARTIAL_READBACK="$partial" \
            BO_SMOKE_SKIA_READBACK_SUBMIT="$readback_submit" \
            BO_SMOKE_VENUS_SYSROOT="$rootfs" BO_SMOKE_SUFFIXE="$suffixe" \
            tools/ci/run_ladybird_browser_host.sh "$dossier/amorce.bin" "$natif" \
            > "$dossier/smoke.log" 2>&1 || code=$?
        rm -- "$dossier/amorce.bin"
        for journal in "serie-browser-host${suffixe}.log" "fixture-browser-host${suffixe}.log" \
                       "surface-browser-host${suffixe}.ppm" "convergence-browser-host${suffixe}.json"; do
            if [[ -f $journal ]]; then mv -- "$journal" "$dossier/"; fi
        done
        rm -f -- "ladybird-browser-host${suffixe}.img"
        etat=ok
        if ((code != 0)) || ! grep -qFx 'LADYBIRD_FUNCTIONAL_SMOKE ok' "$dossier/smoke.log"; then
            etat=echec
            echec=1
        fi
        if [[ $timings == 1 ]] && ! python3 tools/ci/bench_ladybird_readback.py \
            --exige "$backend" "$dossier/serie-browser-host${suffixe}.log" > "$dossier/timings.json"; then
            if [[ $etat == ok ]]; then etat=mesures-incompletes; fi
            echec=1
        fi
        if ! identite | cmp -s "$sortie/entrees.sha256" -; then
            printf '%s\t%s\t%s\tentree-modifiee\n' "$tour" "$essai" "$code" >> "$sortie/verdicts.tsv"
            echo "Campagne invalide : une entrée a changé. Journaux dans $sortie." >&2
            exit 1
        fi
        printf '%s\t%s\t%s\t%s\n' "$tour" "$essai" "$code" "$etat" >> "$sortie/verdicts.tsv"
    done
done
cat "$sortie/verdicts.tsv"
exit "$echec"
