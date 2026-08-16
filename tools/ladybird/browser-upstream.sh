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
VCPKG="$ROOT/third_party/vcpkg-browser-installed/x64-linux"

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

say "== configure Ladybird services-only / Bouchaud =="
cmake -S "$SRC" -B "$BUILD" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_PREFIX_PATH="$VCPKG" \
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

ok "WebContent natif pret : $OUT/WebContent"
ls -lh "$OUT"
