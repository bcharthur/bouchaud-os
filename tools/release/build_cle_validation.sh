#!/usr/bin/env bash
# BOUCHAUD_CLE_VALIDATION_V1 -- construit la cle de VALIDATION du Trigkey.
#
#   build_cle_validation.sh OUT [NATIF_LADYBIRD]
#
# Le MEME systeme que l'image de livraison (tools/release/assemble_systeme.sh,
# tar deterministe), demarre en live depuis une cle USB, noyau compile avec
# `cle-usb-test` : AUCUNE ecriture, vidange, formatage ni effacement vers un
# disque ATA ou NVMe interne ; seule la cle elle-meme (GUID GPT tire ici, grave
# dans le noyau, pose et relu sur l'image) est inscriptible.
#
# Sortie : OUT/usb/bouchaud-trigkey.img (+ .sha256, manifest.json). Aucune
# ecriture de peripherique : l'ecriture sur la cle est le travail de
# tools/windows/Prepare-TrigkeyUSB.ps1 (identite exacte, phrase tapee,
# relecture). Appele tel quel par ce script et par la CI (trigkey-ab-preflight),
# qui demarre le resultat (tools/release/test_cle_validation_qemu.py).
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT=${1:?usage: build_cle_validation.sh OUT [NATIF_LADYBIRD]}
NATIF=${2:-}
rm -rf "$OUT/systeme" "$OUT/systeme.tar" "$OUT/usb"
mkdir -p "$OUT"
bash tools/release/assemble_systeme.sh "$OUT/systeme" ${NATIF:+"$NATIF"} | tee "$OUT/systeme.txt"
python3 tools/release/tar_systeme.py "$OUT/systeme" "$OUT/systeme.tar"
BO_CLE_USB_TEST=1 bash tools/release/build_trigkey_image.sh "$OUT/usb" "$OUT/systeme.tar"
echo "CLE_VALIDATION_CONSTRUITE image=$OUT/usb/bouchaud-trigkey.img sha256=$(cut -d' ' -f1 "$OUT/usb/bouchaud-trigkey.img.sha256") PHYSICAL=NOT_RUN"
