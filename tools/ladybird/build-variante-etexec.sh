#!/usr/bin/env bash
# Relie WebWorker en statique NON-PIE, a cote du binaire normal.
#
# BOUCHAUD_C65_STATIC_PIE_OU_ET_EXEC
#
# # Ce que cette variante teste
#
# Un `-static-pie` glibc se relocalise lui-meme dans `_dl_relocate_static_pie`
# AVANT `main`. L'ELF de WebWorker du run 35912027322 porte :
#
#     taille          186 808 568 octets
#     .rela.dyn         9 731 304 octets
#     RELACOUNT           405 396 relocations R_X86_64_RELATIVE
#
# Quatre cent mille relocations avant la premiere ligne utile, dans un
# intervalle mesure a ~92 s. Un `ET_EXEC` n'a AUCUNE relocation de demarrage :
# la variante supprime la phase au lieu de l'optimiser, ce qui en fait une
# falsification et non un reglage.
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
SRC="${BO_LADYBIRD_SRC:-$ROOT/third_party/ladybird}"
BUILD="$ROOT/third_party/build-ladybird-browser-bouchaud"
SORTIE="${1:-$ROOT/third_party/native-browser-bouchaud}"

if [ ! -d "$BUILD" ]; then
    echo "variante : repertoire de construction absent : $BUILD" >&2
    exit 1
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
    echo "ELF_VARIANTE nom=$nom type=${type:-?} bytes=$taille relasz=${rela:-0} relacount=${relacount:-0}"
}

echo "== etat de reference =="
elf_etat "$SORTIE/WebWorker" baseline

# Le script de preparation remplace son bloc de maniere deterministe : le
# rejouer avec le drapeau bascule les options, le rejouer sans les remet.
echo
echo "== relink WebWorker en ET_EXEC =="
BOUCHAUD_HELPER_ET_EXEC=1 python3 tools/ladybird/prepare-browser-runtime-link.py "$SRC"
cmake -S "$SRC" -B "$BUILD" >/dev/null
cmake --build "$BUILD" --parallel "${BO_JOBS:-$(nproc)}" --target WebWorker

VARIANTE="$BUILD/bin/WebWorker"
[ -f "$VARIANTE" ] || VARIANTE=$(find "$BUILD" -name WebWorker -type f -newer "$SORTIE/WebWorker" | head -1)
if [ -z "${VARIANTE:-}" ] || [ ! -f "$VARIANTE" ]; then
    echo "variante : WebWorker ET_EXEC introuvable apres le lien" >&2
    BOUCHAUD_HELPER_ET_EXEC=0 python3 tools/ladybird/prepare-browser-runtime-link.py "$SRC"
    exit 1
fi

llvm-strip --strip-debug "$VARIANTE" 2>/dev/null || strip --strip-debug "$VARIANTE" || true
cp "$VARIANTE" "$SORTIE/WebWorker.etexec"

echo
echo "== etat de la variante =="
elf_etat "$SORTIE/WebWorker.etexec" etexec

# UN ET_EXEC QUI RESTE ET_DYN N'EST PAS UNE VARIANTE : il faut le refuser ici
# plutot que de mesurer deux fois la meme chose en croyant comparer.
TYPE=$(readelf -h "$SORTIE/WebWorker.etexec" | awk '/Type:/ {print $2}')
if [ "$TYPE" != "EXEC" ]; then
    echo "variante : type ELF=$TYPE, attendu EXEC -- le drapeau n'a pas pris" >&2
    BOUCHAUD_HELPER_ET_EXEC=0 python3 tools/ladybird/prepare-browser-runtime-link.py "$SRC"
    exit 1
fi
if readelf -l "$SORTIE/WebWorker.etexec" | grep -q INTERP; then
    echo "variante : PT_INTERP present dans un binaire cense etre statique" >&2
    exit 1
fi

# L'arbre revient a l'etat de reference : le binaire normal du prochain build
# ne doit pas heriter de l'experience.
echo
echo "== restauration du lien de reference =="
BOUCHAUD_HELPER_ET_EXEC=0 python3 tools/ladybird/prepare-browser-runtime-link.py "$SRC"
cmake -S "$SRC" -B "$BUILD" >/dev/null

echo "VARIANTE_ETEXEC_OK"
