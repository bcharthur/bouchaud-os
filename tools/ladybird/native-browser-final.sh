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
./tools/ladybird/fetch.sh
./tools/ladybird/build-libweb-gen.sh
./tools/ladybird/browser-vcpkg.sh
./tools/ladybird/browser-upstream.sh
./tools/ladybird/webcontent-qemu-bootstrap.sh

echo
printf '\033[32mArtefacts natifs : third_party/native-browser-bouchaud\033[0m\n'
ls -lh third_party/native-browser-bouchaud
