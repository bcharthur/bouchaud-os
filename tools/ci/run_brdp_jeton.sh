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
# # Pourquoi le binaire (`strings`, `nm`) et pas un test unitaire
#
# La decision est prise par le compilateur, pas par le code. Un test qui
# appellerait `arme()` testerait la constante de SA propre compilation, pas
# celle de l'image. Seul le binaire produit peut repondre : ses chaines pour
# les marqueurs et le jeton, ses symboles pour le code des fils.
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

# LE CODE DU FIL, ET NON PLUS SON NOM.
#
# Ce banc comptait la chaine `bouchaud-brdp` : seul le code du serveur la
# portait. Ce n'est plus vrai depuis p18 : la politique de relance des fils
# noyau (`kernel/services/reprise.rs`) tient la table des noms de TOUS les
# services relancables, et cette table est compilee dans toutes les images,
# avec ou sans jeton. Le compte montait donc d'un partout, sans qu'une ligne
# du serveur ait bouge -- et une image sans serveur montrait un « nom du fil ».
#
# Le temoin devient le SYMBOLE du point d'entree du fil. `fil_brdp` n'est
# reference que par la branche armee de `serveur::demarre` : sans jeton, cette
# branche est eliminee et la fonction n'est plus emise du tout. Un nom peut
# figurer dans une table ; une fonction ne figure dans l'image que si son code
# y est.
#
# « Present » veut dire AU MOINS un symbole : une fermeture interne au fil
# porterait le meme prefixe, et son absence ou sa presence n'est pas la
# question posee ici.
#
# FAIL-CLOSED. Sans `nm`, ou si `nm` ne sait pas lire l'image, « absent »
# serait vrai par defaut et l'epreuve la plus importante passerait sans rien
# lire. Deux verrous : l'outil est exige ici, et le fil de TELEMETRIE -- qui
# doit etre present dans les deux images -- sert de temoin positif : si la
# lecture des symboles echoue, c'est lui qui tombe.
#
# `grep -c`, PAS `grep -q`. Sous `pipefail`, `grep -q` sort au premier
# symbole trouve, `nm` recoit SIGPIPE (141) et le tube entier est faux : le
# symbole PRESENT se lisait absent. Le temoin positif l'a attrape a la
# premiere execution -- c'est le meme faux negatif que `run_dhcp_recuperation.sh`.
# `grep -c` lit toute la sortie ; `nm` finit normalement.
present() {
    local n
    n=$(nm "$BIN" 2>/dev/null | grep -c -- "$1" || true)
    if [ "${n:-0}" -gt 0 ]; then echo 1; else echo 0; fi
}
command -v nm >/dev/null 2>&1 || { echo "ECHEC: nm absent, aucun symbole lisible"; exit 1; }

echo "=== 1. construction SANS jeton ==="
if ! env -u BOUCHAUD_DEBUG_TOKEN cargo build >/dev/null 2>&1; then
    echo "ECHEC: la construction sans jeton a echoue"
    exit 1
fi
test -s "$BIN" || { echo "ECHEC: binaire absent"; exit 1; }
SANS_SOMME=$(sha256sum "$BIN" | cut -d' ' -f1)

# LE SERVEUR N'EXISTE PAS. Le code de son fil et son marqueur d'ecoute ne
# sont portes que par la branche armee.
verdict "brdp: code du fil"         0 "$(present 'fil_brdp')"
verdict "brdp: marqueur d'ecoute"   0 "$(compte 'BOUCHAUD_BRDP_ECOUTE')"
verdict "jeton de banc absent"      0 "$(compte "$JETON_BANC")"
# LA TELEMETRIE, ELLE, EST LA.
verdict "telemetrie: code du fil"   1 "$(present 'fil_telemetrie')"
verdict "releve d'amorcage present" 1 "$(compte 'BOUCHAUD_LAB_READY')"

echo "=== 2. construction AVEC jeton ==="
if ! BOUCHAUD_DEBUG_TOKEN="$JETON_BANC" cargo build >/dev/null 2>&1; then
    echo "ECHEC: la construction avec jeton a echoue"
    exit 1
fi
AVEC_SOMME=$(sha256sum "$BIN" | cut -d' ' -f1)

verdict "brdp: code du fil"         1 "$(present 'fil_brdp')"
verdict "brdp: marqueur d'ecoute"   1 "$(compte 'BOUCHAUD_BRDP_ECOUTE')"
verdict "jeton de banc present"     1 "$(compte "$JETON_BANC")"
verdict "telemetrie: code du fil"   1 "$(present 'fil_telemetrie')"
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
verdict "brdp: code du fil"         0 "$(present 'fil_brdp')"

echo
if [ "$echecs" -eq 0 ]; then
    echo "jeton BRDP : toutes les epreuves passent"
else
    echo "jeton BRDP : $echecs epreuve(s) en echec"
fi
[ "$echecs" -eq 0 ]
