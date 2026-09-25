#!/usr/bin/env bash
#
# Le serveur BRDP est-il ABSENT de l'image quand aucun jeton n'est injecte ?
#
# # Ce que ce banc defend
#
# `BOUCHAUD_DEBUG_TOKEN` est lu par `option_env!`, donc a la COMPILATION. Deux
# choses doivent en decouler, et aucune des deux ne se verifie a la lecture :
#
#   1. sans jeton, le serveur n'est pas seulement « desactive » : il n'est PAS
#      DANS L'IMAGE. `demarre_services()` consulte `serveur::arme()`, qui est
#      une constante fausse, et le compilateur elimine l'appel -- donc le fil,
#      donc la boucle d'acceptation. Une image de production ne porte aucun
#      code d'ecoute BRDP, et c'est la seule forme de « desactive » qui ne
#      depende pas d'un drapeau qu'on peut se tromper de sens ;
#
#   2. la telemetrie, elle, est presente DANS LES DEUX. Elle ne demande aucun
#      jeton parce qu'elle n'accepte rien en entree. Si un jour elle disparait
#      des images sans jeton, le canal qui survit a la panne RTL8168 aurait
#      disparu de l'image physique sans que personne le remarque.
#
# # Pourquoi `strings` et pas un test unitaire
#
# La decision est prise par le compilateur, pas par le code. Un test qui
# appellerait `arme()` testerait la constante de SA propre compilation, pas
# celle de l'image. Seul le binaire produit peut repondre.
#
# Le banc ne verifie PAS que l'authentification fonctionne : cela demande un
# vrai echange, et c'est le travail du banc QEMU.
#
# Usage : tools/ci/run_brdp_jeton.sh
set -uo pipefail
cd "$(dirname "$0")/../.."

# UN SECRET EPHEMERE DE BANC, ET RIEN D'AUTRE. Il n'ouvre aucune machine :
# aucune image publiee n'est construite avec, et il ne sert qu'a prouver que
# la presence d'un jeton change l'image. Le garde-fou `verifie-jeton-brdp.py`
# refuse tout autre litteral dans l'arbre.
JETON_BANC="bouchaud-qemu-lab-test"
BIN="target/x86_64-bouchaud_os/debug/bouchaud-os"

echecs=0

verdict() {
    local nom="$1" attendu="$2" obtenu="$3"
    printf '  %-34s attendu=%-8s obtenu=%-8s ' "$nom" "$attendu" "$obtenu"
    if [ "$attendu" = "$obtenu" ]; then
        echo "ok"
    else
        echo "ECHEC"
        echecs=$((echecs + 1))
    fi
}

compte() {
    strings -a "$BIN" | grep -c -- "$1" || true
}

echo "=== 1. construction SANS jeton ==="
if ! env -u BOUCHAUD_DEBUG_TOKEN cargo build >/dev/null 2>&1; then
    echo "ECHEC: la construction sans jeton a echoue"
    exit 1
fi
test -s "$BIN" || { echo "ECHEC: binaire absent"; exit 1; }
SANS_SOMME=$(sha256sum "$BIN" | cut -d' ' -f1)

# LE SERVEUR N'EXISTE PAS. Le nom du fil noyau et le marqueur d'ecoute sont
# des chaines que seul le code du serveur porte.
verdict "brdp: nom du fil"          0 "$(compte 'bouchaud-brdp')"
verdict "brdp: marqueur d'ecoute"   0 "$(compte 'BOUCHAUD_BRDP_ECOUTE')"
verdict "jeton de banc absent"      0 "$(compte "$JETON_BANC")"
# LA TELEMETRIE, ELLE, EST LA.
verdict "telemetrie: nom du fil"    1 "$(compte 'bouchaud-telemetrie')"
verdict "releve d'amorcage present" 1 "$(compte 'BOUCHAUD_LAB_READY')"

echo "=== 2. construction AVEC jeton ==="
if ! BOUCHAUD_DEBUG_TOKEN="$JETON_BANC" cargo build >/dev/null 2>&1; then
    echo "ECHEC: la construction avec jeton a echoue"
    exit 1
fi
AVEC_SOMME=$(sha256sum "$BIN" | cut -d' ' -f1)

verdict "brdp: nom du fil"          1 "$(compte 'bouchaud-brdp')"
verdict "brdp: marqueur d'ecoute"   1 "$(compte 'BOUCHAUD_BRDP_ECOUTE')"
verdict "jeton de banc present"     1 "$(compte "$JETON_BANC")"
verdict "telemetrie: nom du fil"    1 "$(compte 'bouchaud-telemetrie')"
verdict "releve d'amorcage present" 1 "$(compte 'BOUCHAUD_LAB_READY')"

echo "=== 3. le jeton fait bien PARTIE de l'image ==="
# SI LES DEUX SOMMES SONT EGALES, CARGO N'A PAS RECOMPILE. C'est arrive a
# `BOUCHAUD_BUILD_COMMIT` : l'image gardait la valeur figee a la premiere
# construction, et deux archives physiques de suite ont porte « inconnu »
# alors que la variable etait posee. Le suivi d'une variable lue par
# `option_env!` passe par la depinfo de rustc ; ce controle prouve qu'il joue.
if [ "$SANS_SOMME" = "$AVEC_SOMME" ]; then
    echo "  ECHEC: meme empreinte avec et sans jeton -- cargo n'a pas recompile"
    echecs=$((echecs + 1))
else
    echo "  empreintes distinctes                                           ok"
fi

echo "=== 4. l'arbre retrouve son etat sans jeton ==="
env -u BOUCHAUD_DEBUG_TOKEN cargo build >/dev/null 2>&1
verdict "brdp: nom du fil"          0 "$(compte 'bouchaud-brdp')"

echo
if [ "$echecs" -eq 0 ]; then
    echo "jeton BRDP : toutes les epreuves passent"
else
    echo "jeton BRDP : $echecs epreuve(s) en echec"
fi
[ "$echecs" -eq 0 ]
