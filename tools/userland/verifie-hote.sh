#!/bin/bash
# Eprouve l'hote Qt : il peint hors ecran, et l'on relit les pixels.
#
#   ./verifie-hote.sh
#
# `test-moteur.sh` s'arrete a la liste d'affichage — tout ce que Qt en fait
# ensuite n'etait verifie par rien. Les operations de peinture etaient ecrites,
# relues, et jamais executees : une erreur d'ordre d'arguments ou une couleur
# inversee ne se serait vue qu'a l'ecran, sous emulation.
#
# Ce banc compile le **vrai** `hote.cpp` contre le Qt du systeme et le fait
# peindre. Il ne demande ni l'OS, ni framebuffer, ni QuickJS, ni ffmpeg —
# seulement Qt5 et les en-tetes de Python :
#
#   apt install qtbase5-dev python3-dev
#
# Ce qu'il ne prouve pas : que le binaire du navigateur se construit. Cela
# demande Qt statique, CPython embarque, QuickJS et ffmpeg — c'est le travail
# de `build-navigateur.sh`. Ici, les greffons statiques sont neutralises et Qt
# est celui du systeme.

set -e
cd "$(dirname "$0")"

SORTIE=${SORTIE:-build-verifie-hote}
mkdir -p "$SORTIE"

if ! pkg-config --exists Qt5Widgets Qt5Gui Qt5Core; then
    echo "Qt5 de developpement introuvable : apt install qtbase5-dev" >&2
    exit 1
fi
if ! python3-config --includes >/dev/null 2>&1; then
    echo "en-tetes Python introuvables : apt install python3-dev" >&2
    exit 1
fi

INCLUDES_PY=$(python3-config --includes)
LIENS_PY=$(python3-config --ldflags --embed 2>/dev/null || python3-config --ldflags)
INCLUDES_QT=$(pkg-config --cflags Qt5Widgets Qt5Gui Qt5Core)
LIENS_QT=$(pkg-config --libs Qt5Widgets Qt5Gui Qt5Core)

echo "== compilation de l'hote =="
# `-Wall -Wextra` fait partie du contrat : c'est ainsi qu'on a decouvert que la
# mesure de texte ignorait la famille demandee, et mesurait donc dans une police
# quand la peinture en employait une autre.
g++ -std=c++17 -fPIC -Wall -Wextra \
    $INCLUDES_QT $INCLUDES_PY \
    navigateur/verifie_hote.cpp -o "$SORTIE/verifie_hote" \
    $LIENS_QT $LIENS_PY

echo "== peinture hors ecran =="
QT_QPA_PLATFORM=offscreen "$SORTIE/verifie_hote"
