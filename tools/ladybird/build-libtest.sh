#!/bin/bash
# Construit LibTest, le cadre de tests d'upstream, et les tests LibIPC.
#
#   ./tools/ladybird/build-libtest.sh          hote
#   ./tools/ladybird/build-libtest.sh --cible  Bouchaud (static-pie)
#
# ## Pourquoi construire le cadre de tests d'upstream
#
# Parce que Ladybird teste deja LibIPC, et mieux que nous ne le ferions.
# `Tests/LibIPC/` contient huit cas qui visent exactement ce qu'un portage doit
# verifier — le raccrochage du pair, la sortie du fil d'entree/sortie, le
# message arrive juste avant la fin de fichier et qui ne doit pas etre perdu,
# la file d'envoi avec descripteurs.
#
# Ecrire nos propres equivalents aurait produit du code a nous, testant ce que
# nous avions compris de LibIPC. Construire les siens teste ce qu'upstream
# attend de LibIPC — et le jour ou le SHA monte, ces tests montent avec lui.
#
# `LibTest` est petit (trois fichiers) et ne depend que d'AK, LibCore et
# LibFileSystem, tous deja construits.

set -eu
cd "$(dirname "$0")/../.."
RACINE=$(pwd)

LB="$RACINE/third_party/ladybird"
CIBLE=hote
FLAGS_CIBLE=""
if [ "${1:-}" = "--cible" ]; then
    CIBLE=bouchaud
    FLAGS_CIBLE="-static-pie -fPIE"
fi

DEPS="$RACINE/third_party/deps-$CIBLE"
AK="$RACINE/third_party/build-ak-$CIBLE"
GEN="$RACINE/third_party/gen-$CIBLE"
CORE="$RACINE/third_party/build-libcore-$CIBLE"
JS="$RACINE/third_party/build-libjs-$CIBLE"
GC="$RACINE/third_party/build-libgc-$CIBLE"
UNI="$RACINE/third_party/build-libunicode-$CIBLE"
CRYPTO="$RACINE/third_party/build-libcrypto-$CIBLE"
IPC="$RACINE/third_party/build-libipc-$CIBLE"
ICU="$RACINE/third_party/icu-$CIBLE"
RUSTLIB="$RACINE/third_party/build-rust-$CIBLE/x86_64-unknown-linux-gnu/release"
SORTIE="$RACINE/third_party/build-libtest-$CIBLE"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

[ -f "$IPC/libIPC.a" ] || { rouge "LibIPC absent — lancer build-libipc.sh ${1:-}"; exit 1; }

info "== LibTest + tests LibIPC ($CIBLE) =="
mkdir -p "$SORTIE/obj"

CXX=${CXX:-clang++}
# `-DAK_SYSTEM_CACHE_ALIGNMENT_SIZE=64` : voir build-ak.sh.
CXXFLAGS="-std=c++23 -O2 -fno-exceptions -fPIC -DAK_SYSTEM_CACHE_ALIGNMENT_SIZE=64 $FLAGS_CIBLE \
    -I$LB -I$LB/Libraries -I$LB/Tests -I$AK/gen -I$GEN -I$JS/gen \
    -I$DEPS/include -I$ICU/include \
    -Wno-unused-parameter -Wno-unknown-pragmas -Wno-invalid-constexpr \
    -Wno-unqualified-std-cast-call -Wno-user-defined-literals \
    -Wno-unknown-warning-option"

ECHECS=0
compile() {
    local chemin=$1 nom=$2
    local obj="$SORTIE/obj/$nom.o"
    if [ -f "$obj" ] && [ "$obj" -nt "$chemin" ]; then return 0; fi
    if ! $CXX $CXXFLAGS -c "$chemin" -o "$obj" 2> "$SORTIE/obj/$nom.log"; then
        rouge "  ECHEC $nom"
        grep -m3 "error:" "$SORTIE/obj/$nom.log" | sed 's/^/          /'
        ECHECS=$((ECHECS + 1))
    fi
}

# LibTest : la bibliotheque, et `TestMain`/`AssertionHandler` qui forment le
# `main()` — l'equivalent de `$<TARGET_OBJECTS:LibTestMain>` chez upstream.
compile "$LB/Libraries/LibTest/TestSuite.cpp"        TestSuite
compile "$LB/Libraries/LibTest/TestMain.cpp"         TestMain
compile "$LB/Libraries/LibTest/AssertionHandler.cpp" AssertionHandler
# LibFileSystem : une source, reclamee par tout binaire de test.
compile "$LB/Libraries/LibFileSystem/FileSystem.cpp" FileSystem
[ "$ECHECS" -eq 0 ] || { rouge "$ECHECS fichier(s) n'ont pas compile"; exit 1; }

ar rcs "$SORTIE/libTest.a" "$SORTIE/obj/TestSuite.o" "$SORTIE/obj/FileSystem.o"
vert "  ok    libTest.a"

# --- Les tests LibIPC d'upstream --------------------------------------------
#
# `Tests/LibIPC/CMakeLists.txt` reserve `TestTransportSocket` a `UNIX AND NOT
# APPLE` : c'est la branche que Bouchaud presente, on la prend donc aussi.
lie() {
    local nom=$1
    compile "$LB/Tests/LibIPC/$nom.cpp" "$nom"
    [ "$ECHECS" -eq 0 ] || return 1
    info "  LD    $nom"
    # `--start-group` : archives mutuellement recursives (voir build-libcore.sh).
    $CXX $CXXFLAGS "$SORTIE/obj/$nom.o" "$SORTIE/obj/TestMain.o" \
        "$SORTIE/obj/AssertionHandler.o" -o "$SORTIE/$nom" \
        -Wl,--start-group \
        "$SORTIE/libTest.a" "$IPC/libIPC.a" "$JS/libJS.a" \
        "$GC/libGC.a" "$UNI/libUnicode.a" "$CRYPTO/libCryptoMin.a" \
        "$CORE/libCore.a" "$AK/libAK.a" \
        "$RUSTLIB/liblibunicode_rust.a" "$RUSTLIB/liblibregex_rust.a" \
        "$RUSTLIB/libliburl_rust.a" "$RUSTLIB/liblibjs_rust.a" \
        "$RUSTLIB/liblibtextcodec_rust.a" \
        -Wl,--end-group -Wl,--allow-multiple-definition \
        -L"$DEPS/lib" -lsimdjson -ltommath -lpsl -lfmt -lsimdutf -lmimalloc \
        -L"$ICU/lib" -licui18n -licuuc -licudata \
        -lpthread -lm -ldl
    vert "  ok    $nom"
}

lie TestTransportSocket
lie TestConnection

if [ "$CIBLE" = "hote" ]; then
    echo
    for t in TestTransportSocket TestConnection; do
        info "  --- $t ---"
        "$SORTIE/$t" || true
    done
fi
