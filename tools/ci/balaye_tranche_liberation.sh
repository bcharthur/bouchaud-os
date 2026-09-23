#!/usr/bin/env bash
# Balayage de la taille de tranche de `free_frames_lot`.
#
# BOUCHAUD_C45_LA_TENUE_SE_MESURE
#
# Rendre les frames par lots divise le nombre de prises du verrou par la taille
# de tranche -- et multiplie d'autant la duree de CHAQUE prise. Le verrou
# masque les interruptions : une tranche trop grande echange du debit contre de
# la latence.
#
# 256 a ete choisi par raisonnement (« environ cent microsecondes »). Ce script
# le VERIFIE, en mesurant le debit ET la tenue pour chaque taille.
#
# La taille est une constante de compilation : le balayage reconstruit donc le
# noyau a chaque palier. C'est lent et c'est assume -- une mesure qui demande
# quarante minutes une fois vaut mieux qu'un chiffre choisi a vue.
set -euo pipefail
cd "$(dirname "$0")/../.."

SORTIE=${TRANCHE_SORTIE:-$(mktemp -d)}
mkdir -p "$SORTIE"
SORTIE=$(cd "$SORTIE" && pwd)
TAILLES=${TRANCHE_TAILLES:-"32 64 128 256 512"}
SOURCE=src/kernel/memory/virtual.rs

ORIGINE=$(grep -oP 'const TRANCHE_LIBERATION: usize = \K[0-9]+' "$SOURCE")
echo "tranche : valeur d'origine $ORIGINE ; sortie dans $SORTIE"

# La valeur d'origine est remise QUOI QU'IL ARRIVE : un balayage interrompu ne
# doit pas laisser le depot sur un palier arbitraire.
restaure() {
    sed -i "s/^const TRANCHE_LIBERATION: usize = [0-9]*;/const TRANCHE_LIBERATION: usize = $ORIGINE;/" "$SOURCE"
    echo "tranche : $ORIGINE restauree"
}
trap restaure EXIT

printf '%8s %12s %12s %12s %12s %12s\n' \
    taille liberation_us tenue_us pire_us attente_us contentions | tee "$SORTIE/resume.txt"

for TAILLE in $TAILLES; do
    sed -i "s/^const TRANCHE_LIBERATION: usize = [0-9]*;/const TRANCHE_LIBERATION: usize = $TAILLE;/" "$SOURCE"
    if ! cargo bootimage >"$SORTIE/build-$TAILLE.log" 2>&1; then
        echo "tranche : la construction a echoue pour $TAILLE" >&2
        tail -20 "$SORTIE/build-$TAILLE.log" >&2
        exit 1
    fi
    FORK_SORTIE="$SORTIE/run-$TAILLE" timeout 900 ./tools/ci/run_cout_fork.sh \
        >"$SORTIE/bench-$TAILLE.log" 2>&1 || true

    JOURNAL="$SORTIE/run-$TAILLE/propre.log"
    if [ ! -f "$JOURNAL" ]; then
        echo "tranche : aucun journal pour $TAILLE -- rien mesure" >&2
        exit 1
    fi

    # Le plus GROS execve (65540 pages) est celui qui compte : c'est lui qui
    # expose la tenue du verrou.
    LIB=$(grep -ao 'liberation_us=[0-9]*' "$JOURNAL" | sed 's/.*=//' | sort -n | tail -1)
    # `[^\r]*` etait un piege : dans une expression reguliere de base, `\r`
    # dans une classe signifie « ni backslash ni r », pas « pas un retour
    # chariot ». Le motif tronquait donc la ligne au premier `r` -- juste avant
    # `tranche=` -- et toutes les colonnes sortaient vides. `.*` suffit.
    LOT=$(grep -ao 'PERF_FRAME_FREE_BATCH.*' "$JOURNAL" | tr -d '\r' | tail -1)
    champ() { echo "$LOT" | grep -o "$1=[0-9]*" | tail -1 | sed 's/.*=//'; }
    printf '%8s %12s %12s %12s %12s %12s\n' \
        "$TAILLE" "${LIB:--}" "$(champ lock_hold_us)" "$(champ max_lock_hold_us)" \
        "$(champ attente_us)" "$(champ contentions)" | tee -a "$SORTIE/resume.txt"
done

echo
echo "TRANCHE_BALAYAGE_OK ; detail dans $SORTIE"
