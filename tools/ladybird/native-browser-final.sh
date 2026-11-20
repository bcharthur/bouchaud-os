#!/bin/bash
# One entry point for the remaining native Ladybird integration.
set -euo pipefail
cd "$(dirname "$0")/../.."
TARGET=${1:---cible}
if [ "$TARGET" != "--cible" ]; then
    echo "usage: $0 --cible" >&2
    exit 2
fi

printf '\033[1;36m=== Ladybird natif complet / Bouchaud ===\033[0m\n'
python3 tools/verifie-chain-ladybird.py
./tools/ladybird/fetch.sh
./tools/ladybird/build-libweb-gen.sh
./tools/ladybird/browser-upstream.sh
./tools/ladybird/webcontent-qemu-bootstrap.sh

echo
OUT=third_party/native-browser-bouchaud
if [[ ${BOUCHAUD_P10_GPU_VULKAN:-0} == 1 ]]; then OUT="$OUT-vulkan"; fi
printf '\033[32mArtefacts natifs : %s\033[0m\n' "$OUT"
ls -lh "$OUT"
