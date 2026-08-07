#!/bin/sh
# Assemble un bac a sable glibc **dynamique** pour Bouchaud OS.
#
#   ./build-abi-linux.sh
#
# ## Ce que ce script etablit
#
# Bouchaud OS expose l'ABI Linux x86-64. La question qui decide de tout le reste
# est donc : un binaire Linux ordinaire, non recompile, s'y execute-t-il ?
#
# Si oui, amener un moteur web n'est pas le reecrire — c'est honorer les appels
# que son binaire emet, comme le font le linuxulator de FreeBSD, WSL1 et gVisor.
# Si non, le probleme est plus bas, et il faut le savoir avant d'engager quoi
# que ce soit au-dessus.
#
# ## Pourquoi la glibc et pas musl
#
# `build.sh musl-dynamic` eprouve deja le chargement dynamique, mais avec une
# libc qu'on a compilee soi-meme. Les binaires du monde reel — ceux d'Ubuntu, et
# donc ceux de WebKit — sont lies a la **glibc**, qui demande sensiblement plus
# au noyau : `rseq`, `set_robust_list`, une disposition de TLS differente.
#
# ## La contrainte d'adressage
#
# L'OS reserve l'espace utilisateur a partir de 0x400000000000 et refuse un
# binaire lie a l'adresse Linux habituelle. Ubuntu compile en PIE depuis 16.10,
# et `ld-linux-x86-64.so.2` est lui-meme un objet partage : la contrainte tombe
# d'elle-meme. Elle explique en revanche pourquoi un binaire non-PIE echouerait.

set -e
cd "$(dirname "$0")"
OUT=${OUT:-$PWD/out-abi-linux}

rm -rf "$OUT"
mkdir -p "$OUT"

echo "== sonde glibc dynamique =="
gcc -O1 -fPIE -pie -o "$OUT/glibc-probe" abi-linux-probe.c

INTERP=$(readelf -l "$OUT/glibc-probe" | sed -n 's/.*interpreter: \(.*\)\]/\1/p')
echo "   interprete requis : $INTERP"

# Le noyau charge `PT_INTERP` **au chemin exact** inscrit dans le binaire : il
# faut donc reproduire l'arborescence, pas seulement copier les fichiers.
echo "== bibliotheques et interprete =="
copie() {
    dest="$OUT$1"
    mkdir -p "$(dirname "$dest")"
    cp -L "$1" "$dest"
    printf '   %-46s %s\n' "$1" "$(du -h "$dest" | cut -f1)"
}

copie "$INTERP"
ldd "$OUT/glibc-probe" | sed -n 's/.*=> \(\/[^ ]*\).*/\1/p' | while read -r lib; do
    copie "$lib"
done

echo ""
echo "pret dans $OUT ($(du -sh "$OUT" | cut -f1))"
echo "l'ajouter au scenario :   exec /glibc-probe"
