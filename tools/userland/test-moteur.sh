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
#
# Les en-tetes viennent de l'interprete **qui va executer le module**, pas de
# `python3-config`. Sur cette machine les deux divergeaient — en-tetes 3.12,
# interprete 3.11 —, et la consequence n'etait pas une erreur de compilation
# mais quelque chose de bien pire : depuis 3.12, `Py_INCREF(Py_None)` ne fait
# rien parce que `None` y est immortel, tandis que le `Py_DECREF` du 3.11 qui
# executait, lui, decrementait pour de bon. Chaque fonction rendant `None`
# perdait donc une reference, et l'interprete mourait a la sortie sur
# « deallocating None ». Un module compile contre les en-tetes du mauvais
# interprete ne se signale jamais autrement que par ce genre de defaut — tardif,
# non reproductible, et qui accuse le mauvais code.
INCLUDE_PY=$(python3 -c 'import sysconfig; print(sysconfig.get_paths()["include"])')
if [ ! -f "$WORK/bojs.so" ] || [ navigateur/bojs.cpp -nt "$WORK/bojs.so" ]; then
    echo "== compilation de bojs pour le Python local ($INCLUDE_PY) =="
    g++ -O1 -shared -fPIC -DBOJS_MODULE_PARTAGE \
        -I"$INCLUDE_PY" -I"$JS/include" -I navigateur \
        navigateur/bojs.cpp "$JS/lib/libquickjs.a" \
        -o "$WORK/bojs.so"
fi

cp -r navigateur/moteur "$WORK/"
cp navigateur/test_moteur.py "$WORK/"
# `suivi.py` voyage avec : ses verifications protegent le tableau de bord
# lui-meme, qui n'affiche pas d'erreur quand il lit mal — seulement
# « indisponible », ce qui se lit comme une journee calme.
cp navigateur/suivi.py "$WORK/"
# Le serveur de fixtures voyage avec : la moitie du comportement du
# navigateur — redirections, 404, POST, reponse lente — ne se teste
# pas sans lui, et le faire dependre d'Internet le rendait intestable.
cp navigateur/serveur_test.py navigateur/apercu.py "$WORK/"
# La sonde d'ordonnancement voyage avec : elle mesure la latence du chrome
# pendant qu'un **vrai** renderer travaille, et elle a besoin du meme
# echafaudage — module `bojs` compile, moteur, serveur de fixtures.
cp navigateur/ordonnanceur-navigateur.py "$WORK/"
# Les pages temoins voyagent avec les verifications : sans elles, le scenario
# de formulaire se croyait absent et se sautait en silence — une verification
# qui ne s'execute pas est pire qu'une verification qui echoue.
rm -rf "$WORK/tests"
cp -r navigateur/tests "$WORK/"

# Les jalons fonctionnels s'ecrivent dans l'arbre **source**, pas dans la copie
# de travail : celle-ci est effacee a chaque execution, et le tableau de bord
# n'y trouverait rien. Comme `suivi.jsonl`, ils sont versionnes — `git log` sur
# ce fichier raconte quelles capacites sont apparues, et quand.
export BO_JALONS="$PWD/navigateur/tests/jalons.json"

cd "$WORK"
# `BO_SCRIPT` permet de lancer autre chose que les verifications dans le
# meme echafaudage — la sonde d'ordonnancement, par exemple. Dupliquer la
# preparation dans un second script aurait garanti qu'ils divergent.
exec python3 "${BO_SCRIPT:-test_moteur.py}" "$@"
