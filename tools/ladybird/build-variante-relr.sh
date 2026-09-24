#!/usr/bin/env bash
# Relie WebWorker avec une table de relocation RELR, a cote du binaire normal.
#
# BOUCHAUD_C67_LA_TABLE_DE_RELOCATION_DU_DEMARRAGE
#
# # Ce que cette variante teste
#
# Un `-static-pie` glibc applique ses relocations dans
# `_dl_relocate_static_pie`, AVANT `main`. L'ELF de WebWorker du run
# 35912027322 porte 9 731 304 octets de `.rela.dyn` pour 405 396 relocations
# RELATIVE. `-Wl,-z,pack-relative-relocs` encode les memes relocations dans un
# champ de bits `DT_RELR` : sur un temoin local, 26 280 octets deviennent 288.
#
# La variante separe donc deux couts qu'on confondait : PARCOURIR la table
# (9,7 Mio a faire entrer par des fautes de page) et APPLIQUER les 405 396
# ecritures. RELR supprime le premier et garde le second.
#
# # Ce qu'elle remplace
#
# La variante ET_EXEC. Elle est refutee : avec la glibc statique, lier a
# 0x400000000000 echoue (`R_X86_64_PLT32` tronque contre les symboles faibles
# indefinis de `libc.a`). Le detail est dans
# `prepare-browser-runtime-link.py`. RELR, lui, se lie -- et conserve l'ASLR.
#
# # Pourquoi c'est peu couteux
#
# Les objets ne changent pas : seul l'edition de liens change. `ninja
# WebWorker` relie, il ne recompile pas.
#
# # Pourquoi les deux binaires voyagent ensemble
#
# La variance entre coureurs de CI atteint un facteur deux sur le MEME SHA
# (65,9 s contre 129,1 s, runs 35907201865 et 35912027322). Comparer deux runs
# ne prouverait donc rien. Les deux binaires partent dans le meme artefact
# pour etre mesures dans le meme run, sur la meme machine.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$PWD

# LE BON ARBRE, ET PAS CELUI QUE J'AVAIS SUPPOSE.
#
# `browser-upstream.sh` prepare et configure `ladybird-browser-prepared`, pas
# `ladybird` : ce dernier est la copie amont intacte. La premiere version de ce
# script visait `ladybird`, donc patchait un arbre que CMake n'a jamais
# configure -- et `cmake -S` echouait aussitot sur un cache genere depuis une
# autre source. L'etape a dure ZERO seconde au run 35920701144, et
# `WebWorker.relr` n'a jamais existe.
SRC="${BO_LADYBIRD_SRC:-$ROOT/third_party/ladybird-browser-prepared}"
BUILD="$ROOT/third_party/build-ladybird-browser-bouchaud"
SORTIE="${1:-$ROOT/third_party/native-browser-bouchaud}"

if [ ! -d "$SRC" ]; then
    echo "variante : arbre prepare absent : $SRC" >&2
    echo "           c'est celui que CMake configure ; sans lui, le relink" >&2
    echo "           porterait sur une source qui n'a jamais ete construite." >&2
    exit 1
fi
if [ ! -d "$BUILD" ]; then
    echo "variante : repertoire de construction absent : $BUILD" >&2
    exit 1
fi
# La source que le cache CMake connait doit etre CELLE-LA, sinon `cmake -S`
# refusera. Le verifier ici donne un message utile plutot qu'une erreur CMake.
CACHE="$BUILD/CMakeCache.txt"
if [ -f "$CACHE" ]; then
    SRC_CACHE=$(awk -F= '/^CMAKE_HOME_DIRECTORY:/ {print $2}' "$CACHE")
    if [ -n "$SRC_CACHE" ] && [ "$SRC_CACHE" != "$SRC" ]; then
        echo "variante : le cache CMake vient de $SRC_CACHE, pas de $SRC" >&2
        echo "           relier depuis un autre arbre ne mesurerait rien." >&2
        exit 1
    fi
fi
if [ ! -f "$SORTIE/WebWorker" ]; then
    echo "variante : WebWorker de reference absent : $SORTIE/WebWorker" >&2
    echo "           la variante se compare a lui ; sans lui, rien a mesurer." >&2
    exit 1
fi

# L'etat de REFERENCE est releve AVANT tout, pour que la table soit complete
# meme si la variante echoue.
elf_etat() {
    local f=$1 nom=$2
    local taille type rela relacount
    taille=$(stat -c %s "$f")
    type=$(readelf -h "$f" | awk '/Type:/ {print $2}')
    rela=$(readelf -d "$f" 2>/dev/null | awk '/RELASZ/ {gsub(/[^0-9]/,"",$3); print $3}' | head -1)
    relacount=$(readelf -d "$f" 2>/dev/null | awk '/RELACOUNT/ {print $3}' | head -1)
    relrsz=$(readelf -d "$f" 2>/dev/null | awk '/RELRSZ/ {gsub(/[^0-9]/,"",$3); print $3}' | head -1)
    echo "ELF_VARIANTE nom=$nom type=${type:-?} bytes=$taille relasz=${rela:-0} relacount=${relacount:-0} relrsz=${relrsz:-0}"
}

# L'ARBRE REVIENT A L'ETAT DE REFERENCE, MEME SI TOUT ECHOUE.
#
# La premiere version restaurait a la main sur chaque chemin d'erreur, et
# `set -e` sautait par-dessus : un echec de `cmake` laissait l'arbre patche en
# RELR. Un `trap` ne peut pas etre oublie.
restaure_lien() {
    BOUCHAUD_HELPER_RELR=0 python3 tools/ladybird/prepare-browser-runtime-link.py "$SRC" \
        >/dev/null 2>&1 || true
}
trap restaure_lien EXIT

echo "== etat de reference =="
elf_etat "$SORTIE/WebWorker" baseline

# Le script de preparation remplace son bloc de maniere deterministe : le
# rejouer avec le drapeau bascule les options, le rejouer sans les remet.
echo
echo "== relink WebWorker en RELR =="
BOUCHAUD_HELPER_RELR=1 python3 tools/ladybird/prepare-browser-runtime-link.py "$SRC"
cmake -S "$SRC" -B "$BUILD" >/dev/null
cmake --build "$BUILD" --parallel "${BO_JOBS:-$(nproc)}" --target WebWorker

VARIANTE="$BUILD/bin/WebWorker"
[ -f "$VARIANTE" ] || VARIANTE=$(find "$BUILD" -name WebWorker -type f -newer "$SORTIE/WebWorker" | head -1)
if [ -z "${VARIANTE:-}" ] || [ ! -f "$VARIANTE" ]; then
    echo "variante : WebWorker RELR introuvable apres le lien" >&2
    exit 1
fi

llvm-strip --strip-debug "$VARIANTE" 2>/dev/null || strip --strip-debug "$VARIANTE" || true
cp "$VARIANTE" "$SORTIE/WebWorker.relr"

echo
echo "== etat de la variante =="
elf_etat "$SORTIE/WebWorker.relr" relr

# UNE VARIANTE QUI N'A PAS CHANGE N'EST PAS UNE VARIANTE.
#
# Contrairement a ET_EXEC, RELR ne change pas le TYPE de l'ELF : il reste un
# `ET_DYN` static-PIE, et c'est voulu -- l'ASLR est conservee. Le seul signe
# que le drapeau a pris est le deplacement de la table : `DT_RELR` apparait et
# `.rela.dyn` s'effondre. Sans ces deux conditions, on mesurerait deux fois le
# meme binaire en croyant comparer.
if ! readelf -d "$SORTIE/WebWorker.relr" | grep -q "RELRSZ"; then
    echo "variante : aucun DT_RELR -- l'editeur de liens a ignore" >&2
    echo "           -z pack-relative-relocs ; rien a comparer." >&2
    exit 1
fi
RELA_REF=$(readelf -d "$SORTIE/WebWorker" 2>/dev/null | awk '/RELASZ/ {gsub(/[^0-9]/,"",$3); print $3}' | head -1)
RELA_VAR=$(readelf -d "$SORTIE/WebWorker.relr" 2>/dev/null | awk '/RELASZ/ {gsub(/[^0-9]/,"",$3); print $3}' | head -1)
if [ "${RELA_VAR:-0}" -ge "${RELA_REF:-0}" ]; then
    echo "variante : .rela.dyn n'a pas diminue (${RELA_REF:-0} -> ${RELA_VAR:-0})" >&2
    echo "           le drapeau n'a pas pris sur la bonne cible." >&2
    exit 1
fi
if readelf -l "$SORTIE/WebWorker.relr" | grep -q INTERP; then
    echo "variante : PT_INTERP present dans un binaire cense etre statique" >&2
    exit 1
fi

# L'arbre revient a l'etat de reference : le binaire normal du prochain build
# ne doit pas heriter de l'experience.
echo
echo "== restauration du lien de reference =="
restaure_lien
cmake -S "$SRC" -B "$BUILD" >/dev/null

echo "VARIANTE_RELR_OK"
