#!/bin/bash
# BOUCHAUD_LADYBIRD_LIAISON_V1 : verifie l'artefact Ladybird telecharge AVANT
# de le jouer, selon sa provenance dans ce run.
#
#   tete     construit par le job build de CE run : le manifeste doit nommer
#            le HEAD courant (artifact_manifest.py, comportement historique) ;
#   entrees  reutilise d'un run anterieur : le manifeste nomme un autre HEAD,
#            dont les entrees de build doivent etre IDENTIQUES a celles du
#            HEAD courant (tools/ci/ladybird_empreinte.py lie), recalculees ici
#            depuis git -- jamais lues d'une sortie de job.
#
# Dans les deux cas : SHA-256 de chaque binaire et marqueur, empreinte de
# resources/, absence de PT_INTERP, SHA upstream. Toute autre liaison : rouge.
set -euo pipefail
liaison=${1:-}
racine=${2:-native-browser}
manifeste="$racine/BOUCHAUD_ARTIFACT_MANIFEST.json"
case "$liaison" in
    tete)
        python3 tools/ladybird/artifact_manifest.py verify \
            --root "$racine" --upstream third_party/UPSTREAM.md --manifest "$manifeste"
        ;;
    entrees)
        python3 tools/ladybird/artifact_manifest.py verify --allow-head-mismatch \
            --root "$racine" --upstream third_party/UPSTREAM.md --manifest "$manifeste"
        python3 tools/ci/ladybird_empreinte.py lie --manifest "$manifeste"
        ;;
    *)
        echo "LADYBIRD_LIAISON_INCONNUE '${liaison}' (attendu : tete ou entrees)" >&2
        exit 2
        ;;
esac
echo "LADYBIRD_ARTEFACT_VERIFIE liaison=$liaison"
