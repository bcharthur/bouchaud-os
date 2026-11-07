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
va 47 250
double
attente 3
capture atelier-fichiers
origine
va 138 250
double
attente 3
capture atelier-notes
origine
va 47 72
double
attente 3
capture atelier-ladybird
origine
va 226 72
double
attente 3
capture atelier-doom
SCENARIO
)"
BUREAU_ATTENTE_S=${BUREAU_ATTENTE_S:-5} \
BUREAU_GESTES="$GESTES" \
bash tools/ci/run_bureau_capture.sh "$BOOT" "$OUT"
# No false GO: a capture alone does not prove 3rd-party binary availability.
for name in atelier-fichiers atelier-notes atelier-ladybird atelier-doom; do
  test -s "$OUT/$name.png" || { echo "ATELIER_WINDOWS_CAPTURE_FAIL missing=$name" >&2; exit 1; }
done
echo "ATELIER_WINDOWS_CAPTURE_OK files=4"
