#!/bin/bash
# Build the real pinned Ladybird libraries and Services/WebContent in a disposable
# worktree. The executable is linked static-pie so Bouchaud's Linux-ABI userland
# can load it without a dynamic ELF interpreter.
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

rm -rf "$BUILD"
mkdir -p "$BUILD"

export PKG_CONFIG_PATH="$VCPKG/lib/pkgconfig:$VCPKG/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export CMAKE_PREFIX_PATH="$VCPKG${CMAKE_PREFIX_PATH:+:$CMAKE_PREFIX_PATH}"
export CARGO_NET_GIT_FETCH_WITH_CLI=true

# Ladybird upstream utilise du C++ moderne que le GCC 13 de l'image Ubuntu ne
# parse pas correctement (notamment les explicit object parameters / "deducing
# this" utilises par AK::Optional). Le run #27 utilisait implicitement g++ : les
# diagnostics venaient de cc1plus et les options -Wno-* propres a Clang etaient
# rejetees. Le workflow installe deja Clang/LLD ; on les rend donc explicites au
# lieu de laisser CMake choisir le compilateur par defaut.
CLANG=$(command -v clang || true)
CLANGXX=$(command -v clang++ || true)
LLVM_AR=$(command -v llvm-ar || command -v ar)
LLVM_RANLIB=$(command -v llvm-ranlib || command -v ranlib)
[ -n "$CLANG" ] && [ -n "$CLANGXX" ] || {
    echo "Clang/clang++ absents alors que Ladybird les requiert pour ce build" >&2
    exit 1
}

say "Compilateur Ladybird : $($CLANGXX --version | head -n1)"
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
trap restore_bouchaud_cargo_config EXIT
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
    -DCMAKE_AR="$LLVM_AR" \
    -DCMAKE_RANLIB="$LLVM_RANLIB" \
    -DCMAKE_PREFIX_PATH="$VCPKG" \
    -DVCPKG_INSTALLED_DIR="$VCPKG_INSTALLED_ROOT" \
    -D_VCPKG_INSTALLED_DIR="$VCPKG_INSTALLED_ROOT" \
    -DVCPKG_TARGET_TRIPLET="$VCPKG_TRIPLET" \
    -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
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
    -DCMAKE_EXE_LINKER_FLAGS="-static-pie -Wl,--allow-multiple-definition"

say "== build WebContent + services =="
# Building the named targets lets Ninja pull exactly their transitive library
# closure instead of compiling the UI or unrelated test utilities.
cmake --build "$BUILD" --parallel "${BO_JOBS:-$(nproc)}" --target WebContent

# Services are optional here: build those present in this exact upstream SHA.
for target in RequestServer ImageDecoder WebContentCompositor WebWorker; do
    if ninja -C "$BUILD" -t targets all 2>/dev/null | grep -q "^${target}:"; then
        cmake --build "$BUILD" --parallel "${BO_JOBS:-$(nproc)}" --target "$target"
    fi
done

OUT="$ROOT/third_party/native-browser-bouchaud"
rm -rf "$OUT"
mkdir -p "$OUT"
find "$BUILD" -type f \( -name WebContent -o -name RequestServer -o -name ImageDecoder -o -name WebContentCompositor -o -name WebWorker \) -perm -111 -exec cp -f {} "$OUT/" \;

[ -x "$OUT/WebContent" ] || { echo "WebContent non produit" >&2; exit 1; }
file "$OUT/WebContent" | tee "$OUT/file.txt"
if [ -d "$SRC/Base/res" ]; then
    mkdir -p "$OUT/resources"
    cp -a "$SRC/Base/res/." "$OUT/resources/"
fi
if file "$OUT/WebContent" | grep -qi 'dynamically linked'; then
    echo "ERREUR: WebContent contient encore un interpreteur dynamique" >&2
    exit 1
fi

# Le masque Cargo n'est utile que pour le build Ladybird. Restaurer avant la
# sortie normale rend aussi l'etat du checkout explicite pour les etapes CI
# suivantes ; le trap reste la garantie en cas de sortie anticipee.
restore_bouchaud_cargo_config
trap - EXIT

ok "WebContent natif pret : $OUT/WebContent"
ls -lh "$OUT"
