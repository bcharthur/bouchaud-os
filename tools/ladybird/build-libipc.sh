#!/bin/bash
# Construit LibIPC, la couche de communication inter-processus de Ladybird.
#
#   ./tools/ladybird/build-libipc.sh          hote
#   ./tools/ladybird/build-libipc.sh --cible  Bouchaud (static-pie)
#
# ## Pourquoi le vrai LibIPC et non notre protocole
#
# Le navigateur Bouchaud possede deja un protocole artisanal entre le chrome et
# son renderer (`moteur/protocole.py`, 13 genres de messages). Il fonctionne, et
# il a servi. Mais tout ce que Ladybird apporte ensuite — WebContent,
# ImageDecoder, RequestServer, Compositor — parle **LibIPC**. Continuer a
# enrichir le notre reviendrait a ecrire un adaptateur par service, pour finir
# par reimplementer LibIPC en moins bien.
#
# ## Ce que LibIPC demande au systeme
#
# Mesure, pas supposee, dans `TransportSocket.cpp` :
#
#     Core::System::socketpair(AF_LOCAL, SOCK_STREAM)   la paire de sockets
#     Core::System::poll(...)                           l'attente
#     receive_message(..., MSG_DONTWAIT, fds)           recvmsg + SCM_RIGHTS
#     send_message(...)                                 sendmsg + SCM_RIGHTS
#
# Les quatre appels systeme sont routes par Bouchaud (`abi/nr.rs`), et le
# passage de descripteurs par `SCM_RIGHTS` existe deja — c'est ce qui porte le
# partage de surfaces entre le navigateur et son renderer. LibIPC tombe donc sur
# des primitives presentes, ce qui est exactement ce que le portage cherchait a
# obtenir en construisant l'ABI Linux d'abord.
#
# ## Les messages
#
# `Meta/Generators/generate_ipc_definitions.py` — un script **Python**
# d'upstream — transforme un fichier `.ipc` en en-tete C++. Aucune chaine
# supplementaire a introduire : c'est le meme generateur que celui du CMake
# amont, appele de la meme facon.

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
ICU="$RACINE/third_party/icu-$CIBLE"
SORTIE="$RACINE/third_party/build-libipc-$CIBLE"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

[ -f "$CORE/libCore.a" ] || { rouge "LibCore absent — lancer build-libcore.sh ${1:-}"; exit 1; }
[ -f "$JS/libJS.a" ] || { rouge "LibURL absent — il est construit avec LibJS (build-libjs.sh ${1:-})"; exit 1; }

info "== LibIPC ($CIBLE) =="
mkdir -p "$SORTIE/obj" "$SORTIE/gen"

CXX=${CXX:-clang++}
# `-DAK_SYSTEM_CACHE_ALIGNMENT_SIZE=64` : voir build-ak.sh.
CXXFLAGS="-std=c++23 -O2 -fno-exceptions -fPIC -DAK_SYSTEM_CACHE_ALIGNMENT_SIZE=64 $FLAGS_CIBLE \
    -I$LB -I$LB/Libraries -I$AK/gen -I$GEN -I$JS/gen -I$SORTIE/gen \
    -I$DEPS/include -I$ICU/include \
    -Wno-unused-parameter -Wno-unknown-pragmas -Wno-invalid-constexpr \
    -Wno-unqualified-std-cast-call -Wno-user-defined-literals \
    -Wno-unknown-warning-option"

# La liste vient du CMakeLists d'upstream ; la branche retenue est `UNIX`, celle
# que Bouchaud presente. Les variantes Mach (Apple) et Windows sont ecartees par
# le meme `if()` que chez upstream.
SOURCES=$(sed -n '/^set(SOURCES/,/^)/p' "$LB/Libraries/LibIPC/CMakeLists.txt" \
          | grep -oE '[A-Za-z0-9_]+\.cpp')
SOURCES="$SOURCES Attachment.cpp TransportSocket.cpp"

ECHECS=0
OBJETS=""
info "  $(echo "$SOURCES" | wc -w) fichiers"
for src in $SOURCES; do
    obj="$SORTIE/obj/${src%.cpp}.o"
    OBJETS="$OBJETS $obj"
    if [ -f "$obj" ] && [ "$obj" -nt "$LB/Libraries/LibIPC/$src" ]; then
        continue
    fi
    if ! $CXX $CXXFLAGS -c "$LB/Libraries/LibIPC/$src" -o "$obj" 2> "${obj%.o}.log"; then
        rouge "  ECHEC $src"
        grep -m3 "error:" "${obj%.o}.log" | sed 's/^/          /'
        ECHECS=$((ECHECS + 1))
    fi
done
[ "$ECHECS" -eq 0 ] || { rouge "$ECHECS fichier(s) n'ont pas compile"; exit 1; }

ar rcs "$SORTIE/libIPC.a" $OBJETS
vert "  ok    libIPC.a ($(du -h "$SORTIE/libIPC.a" | cut -f1))"

# --- Les endpoints du temoin ------------------------------------------------
#
# Produits par `Meta/Generators/generate_ipc_definitions.py`, le generateur
# d'upstream — le meme que celui qu'appelle `compile_ipc()` dans
# `Meta/CMake/code_generators.cmake`. C'est un script Python : aucune chaine
# supplementaire, et surtout aucune classe Endpoint/Proxy/Stub ecrite a la main.
GENERATEUR="$LB/Meta/Generators/generate_ipc_definitions.py"
for cote in Server Client; do
    entree="$RACINE/tools/ladybird/ipc/BouchaudPortage$cote.ipc"
    sortie="$SORTIE/gen/BouchaudPortage${cote}Endpoint.h"
    if [ ! -f "$sortie" ] || [ "$entree" -nt "$sortie" ]; then
        info "  GEN   BouchaudPortage${cote}Endpoint.h"
        python3 "$GENERATEUR" --input "$entree" --output "$sortie"
    fi
done
vert "  ok    endpoints generes"

info "  CXX   temoin codec"
RUSTLIB="$RACINE/third_party/build-rust-$CIBLE/x86_64-unknown-linux-gnu/release"
GC="$RACINE/third_party/build-libgc-$CIBLE"
UNI="$RACINE/third_party/build-libunicode-$CIBLE"
CRYPTO="$RACINE/third_party/build-libcrypto-$CIBLE"
$CXX $CXXFLAGS "$RACINE/tools/ladybird/libipc-codec-probe.cpp" \
    -o "$SORTIE/libipc-codec-probe" \
    -Wl,--start-group \
    "$SORTIE/libIPC.a" "$JS/libJS.a" "$GC/libGC.a" "$UNI/libUnicode.a" \
    "$CRYPTO/libCryptoMin.a" "$CORE/libCore.a" "$AK/libAK.a" \
    "$RUSTLIB/liblibunicode_rust.a" "$RUSTLIB/liblibregex_rust.a" \
    "$RUSTLIB/libliburl_rust.a" "$RUSTLIB/liblibjs_rust.a" \
    "$RUSTLIB/liblibtextcodec_rust.a" \
    -Wl,--end-group -Wl,--allow-multiple-definition \
    -L"$DEPS/lib" -lsimdjson -ltommath -lpsl -lfmt -lsimdutf -lmimalloc \
    -L"$ICU/lib" -licui18n -licuuc -licudata \
    -lpthread -lm -ldl

vert "  ok    libipc-codec-probe"
if [ "$CIBLE" = "hote" ]; then
    echo
    "$SORTIE/libipc-codec-probe"
fi
