#!/usr/bin/env bash
# Captures AUTHENTIQUES depuis le framebuffer QEMU du noyau; rien de simule.
# bash tools/ci/run_atelier_windows_capture.sh IMAGE [OUT]
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?Usage: run_atelier_windows_capture.sh BOOTIMAGE [SORTIE]}
OUT=${2:-target/atelier-windows-proof}
mkdir -p "$OUT"
# run_bureau_capture change de repertoire pour mkdisk.sh : le chemin IMAGE
# doit donc etre absolu, sinon l'image scenario.img est cherchee sous
# tools/userland/target au lieu de la sortie voulue.
OUT=$(cd "$OUT" && pwd)
# Les icones de bureau sont les memes que celles de src/gui/window.rs.
# Ouvre d'abord Fichiers, Notes puis tente Ladybird et DOOM : meme cadre
# universel, meme si les executables tiers ne sont pas dans cette image.
GESTES="$(cat <<'SCENARIO'
origine
va 389 45
clic
attente 2
capture atelier-terminal
origine
va 55 75
double
attente 2
capture atelier-ladybird
origine
va 225 75
double
attente 2
capture atelier-doom
origine
va 137 165
double
attente 3
capture atelier-notes
origine
va 54 349
double
attente 3
capture atelier-fichiers
SCENARIO
)"
BUREAU_ATTENTE_S=${BUREAU_ATTENTE_S:-5} \
BUREAU_GESTES="$GESTES" \
bash tools/ci/run_bureau_capture.sh "$BOOT" "$OUT"
# No false GO: a capture alone does not prove 3rd-party binary availability.
for name in atelier-terminal atelier-fichiers atelier-notes atelier-ladybird atelier-doom; do
  test -s "$OUT/$name.png" || { echo "ATELIER_WINDOWS_CAPTURE_FAIL missing=$name" >&2; exit 1; }
done
# Les screenshots avec simplement un pointeur deplace ne sont pas des preuves
# d'ouverture de fenetre : comparaison des PPM bruts hors barre haute.
python3 tools/ci/verifie_capture_atelier_fenetres.py "$OUT"
echo "ATELIER_WINDOWS_CAPTURE_OK files=5 preuves=fenetres_reelles"
