#!/bin/bash
# Eprouve le moteur web sur la machine de developpement, sans l'OS.
#
#   ./test-moteur.sh
#
# Le moteur est du Python : analyse HTML, cascade CSS, mise en page, peinture,
# et maintenant JavaScript. Rien de tout cela n'a besoin de Bouchaud OS pour
# tourner — seulement du module `bo`, que l'hote Qt fournit d'habitude. On le
# remplace ici par un bouchon qui mesure le texte a la regle plate, et le moteur
# s'execute tel quel.
#
# L'interet est le delai : reconstruire le navigateur et demarrer l'emulateur
# prend plusieurs minutes, ce script quelques secondes. Ce qu'il ne prouve pas,
# c'est le rendu reel — pour ca, il faut l'OS.

set -e
cd "$(dirname "$0")"
ROOT=$PWD

JS=${JS:-$ROOT/out-quickjs}
WORK=${WORK:-$ROOT/build-test-moteur}
mkdir -p "$WORK"

[ -f "$JS/lib/libquickjs.a" ] || {
    echo "libquickjs.a introuvable — lancer ./build-quickjs.sh" >&2
    exit 1
}

# --- Le module `bojs`, pour le Python de cette machine -----------------------
if [ ! -f "$WORK/bojs.so" ] || [ navigateur/bojs.cpp -nt "$WORK/bojs.so" ]; then
    echo "== compilation de bojs pour le Python local =="
    g++ -O1 -shared -fPIC -DBOJS_MODULE_PARTAGE \
        $(python3-config --includes) -I"$JS/include" -I navigateur \
        navigateur/bojs.cpp "$JS/lib/libquickjs.a" \
        -o "$WORK/bojs.so"
fi

cp -r navigateur/moteur "$WORK/"
cp navigateur/test_moteur.py "$WORK/"

cd "$WORK"
exec python3 test_moteur.py "$@"
