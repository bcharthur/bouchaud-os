#!/bin/bash
# Construit `js`, l'interpreteur JavaScript d'upstream.
#
#   ./tools/ladybird/build-js.sh          hote
#   ./tools/ladybird/build-js.sh --cible  Bouchaud (static-pie)
#
# ## Pourquoi cet executable et pas seulement notre temoin
#
# `libjs-probe` est **notre** code. Il prouve que la chaine tient, et il a servi
# a cela, mais un programme ecrit pour passer ses propres verifications ne
# demontre pas grand-chose sur la bibliotheque : c'est nous qui avons choisi ce
# qu'il appelle.
#
# `Utilities/js.cpp` est le programme d'upstream — 1 151 lignes, la boucle
# interactive, l'execution de fichiers, l'affichage des valeurs, la gestion des
# erreurs de syntaxe et des exceptions non rattrapees. Le construire sans le
# modifier, et le voir executer du JavaScript, est une preuve d'une autre nature :
# elle porte sur ce qu'upstream attend de sa propre bibliotheque, pas sur ce que
# nous savions deja faire marcher.
#
# C'est la condition posee avant d'aborder LibWeb : un executable LibJS
# reellement fonctionnel, et non une sonde.
#
# ## libedit
#
# `js` lie `PkgConfig::LIBEDIT` sur toute cible non-Windows : la boucle
# interactive passe par `editline/readline.h`. `vcpkg.json` epingle la version
# `2024-08-08` ; la distribution fournit `3.1-20230828`, la meme serie a un an
# pres. C'est l'un des rares cas ou l'ecart passe — verifie en construisant, pas
# suppose, conformement a la regle que le portage s'est donnee apres ICU et
# libtommath.
#
# `pkg-config --static` et non `pkg-config` : le premier ajoute `Libs.private`,
# c'est-a-dire `-ltermcap` et `-lbsd`. En liaison dynamique ces dependances se
# resolvent d'elles-memes par le `DT_NEEDED` de `libedit.so` ; en `-static-pie`
# personne ne les apporte, et l'edition de liens echoue sur `tputs` — un symbole
# de terminfo dont rien dans le message ne dit qu'il vient de libedit.

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
GC="$RACINE/third_party/build-libgc-$CIBLE"
UNI="$RACINE/third_party/build-libunicode-$CIBLE"
CRYPTO="$RACINE/third_party/build-libcrypto-$CIBLE"
JS="$RACINE/third_party/build-libjs-$CIBLE"
ICU="$RACINE/third_party/icu-$CIBLE"
RUSTLIB="$RACINE/third_party/build-rust-$CIBLE/x86_64-unknown-linux-gnu/release"
SORTIE="$RACINE/third_party/build-js-$CIBLE"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

[ -f "$JS/libJS.a" ] || { rouge "LibJS absent — lancer build-libjs.sh ${1:-}"; exit 1; }
pkg-config --exists libedit || { rouge "libedit absent — apt install libedit-dev"; exit 1; }

info "== js ($CIBLE) — l'interpreteur d'upstream =="

mkdir -p "$SORTIE/obj"

CXX=${CXX:-clang++}
CXXFLAGS="-std=c++23 -O2 -fno-exceptions -fPIC $FLAGS_CIBLE \
    -I$LB -I$LB/Libraries -I$AK/gen -I$GEN -I$JS/gen \
    -I$DEPS/include -I$ICU/include $(pkg-config --cflags libedit) \
    -Wno-unused-parameter -Wno-unknown-pragmas -Wno-invalid-constexpr \
    -Wno-unqualified-std-cast-call -Wno-user-defined-literals \
    -Wno-unknown-warning-option"

# LibMain : une source. Elle fournit le `main()` qui appelle `serenity_main()`,
# la convention d'upstream pour que tout programme rende un `ErrorOr<int>`.
ECHECS=0
for cible in "LibMain/Main.cpp:main" "../Utilities/js.cpp:js"; do
    src=${cible%%:*}
    nom=${cible##*:}
    obj="$SORTIE/obj/$nom.o"
    if [ -f "$obj" ] && [ "$obj" -nt "$LB/Libraries/$src" ]; then
        continue
    fi
    info "  CXX   $(basename "$src")"
    if ! $CXX $CXXFLAGS -c "$LB/Libraries/$src" -o "$obj" 2> "$SORTIE/obj/$nom.log"; then
        rouge "  ECHEC $src"
        grep -m3 "error:" "$SORTIE/obj/$nom.log" | sed 's/^/          /'
        ECHECS=$((ECHECS + 1))
    fi
done
[ "$ECHECS" -eq 0 ] || { rouge "$ECHECS fichier(s) n'ont pas compile"; exit 1; }

# `--start-group` : archives mutuellement recursives (C++ <-> caisses Rust).
# Voir build-libcore.sh pour l'explication complete.
info "  LD    js"
$CXX $CXXFLAGS "$SORTIE/obj/js.o" "$SORTIE/obj/main.o" -o "$SORTIE/js" \
    -Wl,--start-group \
    "$JS/libJS.a" "$GC/libGC.a" "$UNI/libUnicode.a" "$CRYPTO/libCryptoMin.a" \
    "$CORE/libCore.a" "$AK/libAK.a" \
    "$RUSTLIB/liblibunicode_rust.a" "$RUSTLIB/liblibregex_rust.a" \
    "$RUSTLIB/libliburl_rust.a" "$RUSTLIB/liblibjs_rust.a" \
    "$RUSTLIB/liblibtextcodec_rust.a" \
    -Wl,--end-group \
    -Wl,--allow-multiple-definition \
    -L"$DEPS/lib" -lsimdjson -ltommath -lpsl -lfmt -lsimdutf -lmimalloc \
    -L"$ICU/lib" -licui18n -licuuc -licudata \
    $(pkg-config --static --libs libedit) -lpthread -lm -ldl

vert "  ok    js ($(du -h "$SORTIE/js" | cut -f1))"

if [ "$CIBLE" = "hote" ]; then
    echo
    info "  --- js d'upstream, sur un vrai fichier ---"
    "$SORTIE/js" "$RACINE/tools/ladybird/temoin.js"
fi
