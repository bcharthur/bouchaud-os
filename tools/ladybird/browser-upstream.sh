#!/bin/bash
# Build the real pinned Ladybird libraries and Services/WebContent in a disposable
# worktree. Only the runtime services are linked static-pie for Bouchaud's
# Linux-ABI userland; build-time generators remain normal Ubuntu executables.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$(pwd)
LB="$ROOT/third_party/ladybird"
SRC="$ROOT/third_party/ladybird-browser-src"
BUILD="$ROOT/third_party/build-ladybird-browser-bouchaud"
VCPKG_INSTALLED_ROOT="$ROOT/third_party/vcpkg-browser-installed"
VCPKG_TRIPLET="x64-linux"
VCPKG="$VCPKG_INSTALLED_ROOT/$VCPKG_TRIPLET"

say(){ printf '\033[1;36m%s\033[0m\n' "$*"; }
ok(){ printf '\033[32m%s\033[0m\n' "$*"; }

./tools/ladybird/fetch.sh
./tools/ladybird/fetch.sh --verifie
./tools/ladybird/browser-vcpkg.sh

# A real worktree keeps the source pinned and avoids copying several GiB.
if [ -e "$SRC/.git" ]; then
    git -C "$LB" worktree remove --force "$SRC" >/dev/null 2>&1 || true
fi
rm -rf "$SRC"
git -C "$LB" worktree prune
git -C "$LB" worktree add --force --detach "$SRC" HEAD >/dev/null
python3 tools/ladybird/prepare-browser-source.py "$SRC"
python3 tools/ladybird/prepare-m9-source.py "$SRC"
python3 tools/ladybird/prepare-browser-runtime-link.py "$SRC"

# Do not destroy a build directory restored by GitHub Actions. CMake/Ninja can
# reconfigure it in place and ccache catches recompilations forced only by fresh
# worktree mtimes. Set BO_CLEAN_LADYBIRD_BUILD=1 for deliberate clean rebuilds.
if [ "${BO_CLEAN_LADYBIRD_BUILD:-0}" = "1" ]; then
    say "nettoyage explicite du build Ladybird"
    rm -rf "$BUILD"
fi
mkdir -p "$BUILD"

export PKG_CONFIG_PATH="$VCPKG/lib/pkgconfig:$VCPKG/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export CMAKE_PREFIX_PATH="$VCPKG${CMAKE_PREFIX_PATH:+:$CMAKE_PREFIX_PATH}"
export CARGO_NET_GIT_FETCH_WITH_CLI=true

# Ladybird upstream utilise LLVM 20 dans son environnement Ubuntu. Clang 18
# parse le C++ moderne d'AK mais segfault dans LibWasm/BytecodeInterpreter.cpp
# pendant la generation LLVM IR (run #36, exit 139). Ne plus prendre le `clang`
# non versionne du runner : forcer exactement la generation LLVM supportee par
# le snapshot Ladybird epingle.
LLVM_VERSION="${BO_LLVM_VERSION:-20}"
CLANG=$(command -v "clang-${LLVM_VERSION}" || true)
CLANGXX=$(command -v "clang++-${LLVM_VERSION}" || true)
LLVM_AR=$(command -v "llvm-ar-${LLVM_VERSION}" || true)
LLVM_RANLIB=$(command -v "llvm-ranlib-${LLVM_VERSION}" || true)
LLD=$(command -v "ld.lld-${LLVM_VERSION}" || true)
CCACHE=$(command -v ccache || true)

for tool in "$CLANG" "$CLANGXX" "$LLVM_AR" "$LLVM_RANLIB" "$LLD" "$CCACHE"; do
    [ -n "$tool" ] || {
        echo "Toolchain LLVM ${LLVM_VERSION}/ccache incomplete ; le workflow doit installer clang/lld/llvm-${LLVM_VERSION} et ccache" >&2
        exit 1
    }
done

# `-fuse-ld=lld` cherche aussi son linker dans PATH. Mettre le bindir reel de
# clang-20 devant /usr/bin evite qu'un ld.lld 18 non versionne soit repris.
LLVM_BINDIR=$(dirname "$(readlink -f "$CLANGXX")")
export PATH="$LLVM_BINDIR:$PATH"

CLANG_MAJOR=$("$CLANGXX" -dumpversion | cut -d. -f1)
if [ "$CLANG_MAJOR" -lt "$LLVM_VERSION" ]; then
    echo "Clang trop ancien : $($CLANGXX --version | head -n1), LLVM ${LLVM_VERSION}+ requis" >&2
    exit 1
fi

# ccache is the durable layer. Even if a fresh Ladybird worktree gives every
# source a newer timestamp and Ninja decides to rebuild it, unchanged compile
# commands are served from this cache instead of spending another 90 minutes.
export CCACHE_DIR="${CCACHE_DIR:-$HOME/.cache/ccache}"
export CCACHE_BASEDIR="$ROOT"
export CCACHE_NOHASHDIR=true
mkdir -p "$CCACHE_DIR"
ccache --max-size="${BO_CCACHE_MAXSIZE:-4G}" >/dev/null
ccache --zero-stats >/dev/null || true

say "Compilateur Ladybird : $($CLANGXX --version | head -n1)"
say "Linker Ladybird      : $($LLD --version | head -n1)"
say "ccache Ladybird      : $CCACHE ($CCACHE_DIR)"
# Preflight volontairement minuscule : il teste exactement la famille de syntaxe
# qui a casse AK::Optional au run #27. Si l'image runner fournit un Clang trop
# ancien, on echoue ici en quelques secondes plutot qu'apres la generation Ninja.
cat > "$BUILD/clang-explicit-this.cpp" <<'EOF_CLANG_TEST'
struct Probe {
    template<class Self>
    constexpr int value(this Self& self) { (void)self; return 0; }
};
int main() { Probe p; return p.value(); }
EOF_CLANG_TEST
"$CLANGXX" -std=c++23 -fsyntax-only "$BUILD/clang-explicit-this.cpp"

# Le depot Bouchaud possede volontairement une configuration Cargo bare-metal
# a sa racine : cible JSON x86_64-bouchaud_os + `build-std = core,alloc,...`.
# Les commandes Cargo lancees par CMake depuis un build situe sous ce depot
# remontent l'arborescence et heritent de cette configuration. Pour Ladybird,
# c'est incorrect : upstream demande son propre toolchain hote et doit utiliser
# la std precompilee de x86_64-unknown-linux-gnu. L'heritage de build-std faisait
# charger simultanement le `core` du toolchain et un `core` reconstruit, d'ou
# E0152 duplicate lang item `sized` dans LibUnicode.
#
# On epingle donc explicitement le toolchain declare par Ladybird et on masque
# temporairement UNIQUEMENT la config Cargo du noyau pendant configure/build.
# Un trap la restaure meme si CMake/Ninja/Cargo echoue.
LADYBIRD_RUST_TOOLCHAIN=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$SRC/rust-toolchain.toml" | head -n1)
[ -n "$LADYBIRD_RUST_TOOLCHAIN" ] || {
    echo "toolchain Rust Ladybird introuvable dans $SRC/rust-toolchain.toml" >&2
    exit 1
}
say "Rust Ladybird : $LADYBIRD_RUST_TOOLCHAIN"
rustup toolchain install "$LADYBIRD_RUST_TOOLCHAIN" --profile minimal >/dev/null
export RUSTUP_TOOLCHAIN="$LADYBIRD_RUST_TOOLCHAIN"

ROOT_CARGO_CONFIG="$ROOT/.cargo/config.toml"
ROOT_CARGO_CONFIG_SAVED="$ROOT/.cargo/config.toml.bouchaud-kernel-saved"
restore_bouchaud_cargo_config() {
    if [ -f "$ROOT_CARGO_CONFIG_SAVED" ]; then
        mv -f "$ROOT_CARGO_CONFIG_SAVED" "$ROOT_CARGO_CONFIG"
    fi
}
report_ccache() {
    ccache --show-stats || true
}
trap 'report_ccache; restore_bouchaud_cargo_config' EXIT
if [ -f "$ROOT_CARGO_CONFIG" ]; then
    mv "$ROOT_CARGO_CONFIG" "$ROOT_CARGO_CONFIG_SAVED"
fi

# Preflight : le rustc vu par CMake/Cargo doit maintenant etre celui de
# Ladybird, et aucune cible Bouchaud ne doit etre injectee par l'environnement.
rustc -vV | tee "$BUILD/rustc-version.txt"
rustc -vV | grep -F "release: $LADYBIRD_RUST_TOOLCHAIN"
if env | grep -q '^CARGO_BUILD_TARGET=.*bouchaud'; then
    echo "CARGO_BUILD_TARGET Bouchaud fuit encore dans le build Ladybird" >&2
    exit 1
fi

# Plusieurs configs CMake produites par vcpkg (notamment harfbuzzConfig.cmake)
# ne sont pas totalement relocatables : elles reconstruisent leurs chemins avec
# VCPKG_INSTALLED_DIR/_VCPKG_INSTALLED_DIR + VCPKG_TARGET_TRIPLET. Comme nous
# consommons les archives vcpkg depuis un CMake Ladybird externe au toolchain
# vcpkg, ces variables seraient sinon vides et HarfBuzz annoncerait par exemple
# `//include/harfbuzz`.
#
# On fournit donc explicitement le contexte de l'install root sans activer le
# toolchain vcpkg ni son mode manifeste : la resolution/reconstruction des 78
# dependances reste entierement sous le controle de browser-vcpkg.sh.
[ -d "$VCPKG/include/harfbuzz" ] || {
    echo "headers HarfBuzz absents: $VCPKG/include/harfbuzz" >&2
    exit 1
}
[ -f "$VCPKG/lib/libharfbuzz.a" ] || {
    echo "archive HarfBuzz absente: $VCPKG/lib/libharfbuzz.a" >&2
    exit 1
}

# Certains outils generateurs de Lagom/Ladybird lient mimalloc par son nom
# (`-lmimalloc`) au lieu d'une archive absolue. CMAKE_PREFIX_PATH permet bien de
# trouver le package, mais n'ajoute pas a lui seul le repertoire a la recherche
# link-time de `ld.lld`. LIBRARY_PATH + CMAKE_LIBRARY_PATH donnent ce chemin aux
# outils hote sans leur imposer le format static-pie de Bouchaud.
MIMALLOC_ARCHIVE="$VCPKG/lib/libmimalloc.a"
[ -f "$MIMALLOC_ARCHIVE" ] || {
    echo "archive mimalloc absente: $MIMALLOC_ARCHIVE" >&2
    find "$VCPKG/lib" -maxdepth 1 -iname '*mimalloc*' -print >&2 || true
    exit 1
}
export LIBRARY_PATH="$VCPKG/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"

say "== configure Ladybird services-only / Bouchaud =="
printf '  vcpkg install root : %s\n' "$VCPKG_INSTALLED_ROOT"
printf '  vcpkg triplet      : %s\n' "$VCPKG_TRIPLET"
printf '  rust toolchain     : %s\n' "$LADYBIRD_RUST_TOOLCHAIN"
printf '  C compiler         : %s\n' "$CLANG"
printf '  C++ compiler       : %s\n' "$CLANGXX"
cmake -S "$SRC" -B "$BUILD" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_C_COMPILER="$CLANG" \
    -DCMAKE_CXX_COMPILER="$CLANGXX" \
    -DCMAKE_C_COMPILER_LAUNCHER="$CCACHE" \
    -DCMAKE_CXX_COMPILER_LAUNCHER="$CCACHE" \
    -DCMAKE_AR="$LLVM_AR" \
    -DCMAKE_RANLIB="$LLVM_RANLIB" \
    -DCMAKE_LINKER="$LLD" \
    -DCMAKE_PREFIX_PATH="$VCPKG" \
    -DCMAKE_LIBRARY_PATH="$VCPKG/lib" \
    -DVCPKG_INSTALLED_DIR="$VCPKG_INSTALLED_ROOT" \
    -D_VCPKG_INSTALLED_DIR="$VCPKG_INSTALLED_ROOT" \
    -DVCPKG_TARGET_TRIPLET="$VCPKG_TRIPLET" \
    -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
    -DCMAKE_CXX_FLAGS="-Wno-error=invalid-constexpr" \
    -DBUILD_SHARED_LIBS=OFF \
    -DBUILD_TESTING=OFF \
    -DENABLE_GUI_TARGETS=ON \
    -DBOUCHAUD_SERVICES_ONLY=ON \
    -DBOUCHAUD_PORT=ON \
    -DENABLE_CLANG_PLUGINS=OFF \
    -DENABLE_LTO_FOR_RELEASE=OFF \
    -DENABLE_INSTALL_FREEDESKTOP_FILES=OFF \
    -DLADYBIRD_ENABLE_CPPTRACE=OFF \
    -DLAGOM_USE_LINKER=lld \
    -DCMAKE_EXE_LINKER_FLAGS="-L$VCPKG/lib"

# Ladybird construit plusieurs executables uniquement pour LA MACHINE HOTE.
# Le run #39 a montre que generate_interpreter_layout se liait puis segfaultait
# car notre ancien -static-pie global contaminait aussi ce generateur Ubuntu.
# Valider explicitement la separation host/target avant de lancer les 2762 jobs.
say "== preflight generate_interpreter_layout (host Ubuntu) =="
cmake --build "$BUILD" --parallel "${BO_JOBS:-$(nproc)}" --target generate_interpreter_layout
GEN_LAYOUT="$BUILD/bin/generate_interpreter_layout"
[ -x "$GEN_LAYOUT" ] || { echo "generate_interpreter_layout non produit" >&2; exit 1; }
file "$GEN_LAYOUT" | tee "$BUILD/generate-interpreter-layout.file.txt"
if ! readelf -l "$GEN_LAYOUT" | grep -q 'INTERP'; then
    echo "ERREUR: generate_interpreter_layout n'est pas un executable hote dynamique; un flag Bouchaud fuit encore" >&2
    readelf -l "$GEN_LAYOUT" >&2 || true
    exit 1
fi
HOST_LAYOUT="$BUILD/generate-interpreter-layout-preflight.conf"
if ! (ulimit -c 0; "$GEN_LAYOUT" > "$HOST_LAYOUT"); then
    status=$?
    echo "ERREUR: generate_interpreter_layout plante sur le runner (status=$status)" >&2
    ldd "$GEN_LAYOUT" >&2 || true
    exit "$status"
fi
[ -s "$HOST_LAYOUT" ] || { echo "generate_interpreter_layout a produit un fichier vide" >&2; exit 1; }
grep -q '^const OBJECT_SIZE = ' "$HOST_LAYOUT" || {
    echo "sortie generate_interpreter_layout invalide (OBJECT_SIZE absent)" >&2
    head -n 40 "$HOST_LAYOUT" >&2 || true
    exit 1
}
ok "generateur LibJS hote valide"

say "== build WebContent + services =="
# Pendant le portage on veut decouvrir en un seul run toutes les incompatibilites
# independantes, au lieu de faire un cycle CI par fichier. Ninja -k 0 continue
# donc tant qu'il reste une cible qui peut etre construite. Le build reste en
# echec a la fin si des erreurs subsistent : on ne masque pas les vraies erreurs,
# on les collecte simplement en lot.
#
# invalid-constexpr est temporairement demote de -Werror ci-dessus : le run #30
# montre que ce diagnostic Clang touche AK::Optional<Utf16String> dans l'upstream
# epingle. Il reste visible comme warning, sans couper la traversee Ninja.
cmake --build "$BUILD" --parallel "${BO_JOBS:-$(nproc)}" --target WebContent -- -k 0

# Services de processus utiles au chemin CPU-only. Le target GPU upstream est
# `Compositor`, pas `WebContentCompositor`; on ne le construit volontairement
# pas ici car Bouchaud vise Skia CPU -> shared surface -> WM, sans ANGLE/OpenGL.
for target in RequestServer ImageDecoder WebWorker; do
    if ninja -C "$BUILD" -t targets all 2>/dev/null | grep -q "^${target}:"; then
        cmake --build "$BUILD" --parallel "${BO_JOBS:-$(nproc)}" --target "$target" -- -k 0
    fi
done

OUT="$ROOT/third_party/native-browser-bouchaud"
rm -rf "$OUT"
mkdir -p "$OUT"
find "$BUILD" -type f \( -name WebContent -o -name RequestServer -o -name ImageDecoder -o -name WebWorker \) -perm -111 -exec cp -f {} "$OUT/" \;

[ -x "$OUT/WebContent" ] || { echo "WebContent non produit" >&2; exit 1; }
[ -x "$OUT/RequestServer" ] || { echo "RequestServer non produit (M9 requis)" >&2; exit 1; }
printf 'Bouchaud Ladybird M9 capable
pinned=%s
' "$(git -C "$LB" rev-parse HEAD)" > "$OUT/M9_CAPABLE"
file "$OUT/WebContent" | tee "$OUT/file.txt"
if [ -d "$SRC/Base/res" ]; then
    mkdir -p "$OUT/resources"
    cp -a "$SRC/Base/res/." "$OUT/resources/"
fi

# `file` peut varier selon la version du runner; readelf est l'invariant utile
# pour Bouchaud : aucun PT_INTERP ne doit demander ld-linux au demarrage.
for runtime in WebContent RequestServer ImageDecoder WebWorker; do
    [ -x "$OUT/$runtime" ] || continue
    file "$OUT/$runtime" | tee "$OUT/$runtime.file.txt"
    if readelf -l "$OUT/$runtime" | grep -q 'INTERP'; then
        echo "ERREUR: $runtime contient encore un interpreteur ELF dynamique" >&2
        readelf -l "$OUT/$runtime" >&2 || true
        exit 1
    fi
done

# Le masque Cargo n'est utile que pour le build Ladybird. Restaurer avant la
# sortie normale rend aussi l'etat du checkout explicite pour les etapes CI
# suivantes ; le trap reste la garantie en cas de sortie anticipee.
report_ccache
restore_bouchaud_cargo_config
trap - EXIT

ok "WebContent natif pret : $OUT/WebContent"
ls -lh "$OUT"
