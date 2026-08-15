#!/bin/bash
# Construit LibGfx upstream complet pour le premier backend Bouchaud :
# Skia CPU -> Gfx::Bitmap -> pixels BGRA en ring 3.
#
# Ce script compile la liste de SOURCES du CMakeLists upstream epingle. Les
# sources conditionnelles Vulkan/Direct3D/Metal ne sont pas ajoutees : Bouchaud
# n'expose aucun GPU dans cette etape M6.

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
IPC="$RACINE/third_party/build-libipc-$CIBLE"
GC="$RACINE/third_party/build-libgc-$CIBLE"
UNI="$RACINE/third_party/build-libunicode-$CIBLE"
CRYPTO="$RACINE/third_party/build-libcrypto-$CIBLE"
ICU="$RACINE/third_party/icu-$CIBLE"
RUST="$RACINE/third_party/build-rust-$CIBLE/x86_64-unknown-linux-gnu/release"
RUSTGFXROOT="$RACINE/third_party/build-rust-gfx-$CIBLE"
RUSTGFX="$RUSTGFXROOT/x86_64-unknown-linux-gnu/release"
COMP="$RACINE/third_party/build-libcompress-$CIBLE"
VCPKG="$RACINE/third_party/vcpkg-gfx-installed/x64-linux"
SORTIE="$RACINE/third_party/build-libgfx-$CIBLE"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

for f in \
    "$AK/libAK.a" \
    "$CORE/libCore.a" \
    "$JS/libJS.a" \
    "$IPC/libIPC.a" \
    "$GC/libGC.a" \
    "$UNI/libUnicode.a" \
    "$CRYPTO/libCryptoMin.a" \
    "$COMP/libCompress.a" \
    "$RUSTGFX/liblibgfx_rust.a" \
    "$VCPKG/lib/libskia.a"
do
    [ -f "$f" ] || { rouge "dependance M6 absente : $f"; exit 1; }
done

mkdir -p "$SORTIE/obj"
CXX=${CXX:-clang++}
CXXFLAGS="-std=c++23 -O2 -fno-exceptions -fPIC \
    -DAK_SYSTEM_CACHE_ALIGNMENT_SIZE=64 $FLAGS_CIBLE \
    -I$LB -I$LB/Libraries \
    -I$AK/gen -I$GEN -I$JS/gen -I$IPC/gen \
    -I$RUSTGFXROOT/gen -I$RUSTGFXROOT/gen/LibGfx \
    -I$DEPS/include -I$ICU/include \
    -I$VCPKG/include -I$VCPKG/include/skia \
    -Wno-unused-parameter -Wno-unknown-pragmas -Wno-invalid-constexpr \
    -Wno-unqualified-std-cast-call -Wno-user-defined-literals \
    -Wno-unknown-warning-option"

# La section set(SOURCES ...) de l'upstream contient le backend portable CPU.
# Les ajouts conditionnels Apple/DirectX/Vulkan/GlobalFontConfig sont hors de
# cette section et ne sont donc pas pris ici.
SOURCES=$(sed -n '/^set(SOURCES/,/^)/p' "$LB/Libraries/LibGfx/CMakeLists.txt" \
          | grep -oE '[A-Za-z0-9_/]+\.cpp')

NB=$(echo "$SOURCES" | wc -w)
info "== LibGfx ($CIBLE) : $NB sources upstream =="

rm -f "$SORTIE"/obj/*.echec
OBJETS=""
PARALLELE=${BO_JOBS:-$(nproc)}
EN_VOL=0

for src in $SOURCES; do
    nom=$(echo "${src%.cpp}" | tr / -)
    obj="$SORTIE/obj/$nom.o"
    OBJETS="$OBJETS $obj"

    if [ -f "$obj" ] && [ "$obj" -nt "$LB/Libraries/LibGfx/$src" ]; then
        continue
    fi

    (
        if ! $CXX $CXXFLAGS -c "$LB/Libraries/LibGfx/$src" -o "$obj" \
            2> "${obj%.o}.log"; then
            touch "${obj%.o}.echec"
        fi
    ) &

    EN_VOL=$((EN_VOL + 1))
    if [ "$EN_VOL" -ge "$PARALLELE" ]; then
        wait
        EN_VOL=0
    fi
done
wait

ECHECS=0
for marque in "$SORTIE"/obj/*.echec; do
    [ -e "$marque" ] || break
    base=${marque%.echec}
    rouge "ECHEC $(basename "$base")"
    grep -m4 "error:" "$base.log" | sed 's/^/        /' || true
    ECHECS=$((ECHECS + 1))
done
[ "$ECHECS" -eq 0 ] || {
    rouge "$ECHECS source(s) LibGfx n'ont pas compile"
    rouge "logs : $SORTIE/obj/*.log"
    exit 1
}

ar rcs "$SORTIE/libGfx.a" $OBJETS
vert "libGfx.a pret ($(du -h "$SORTIE/libGfx.a" | cut -f1))"

# Toutes les archives vcpkg sont dans le meme --start-group : cela laisse le
# linker ne tirer que les objets effectivement references tout en resolvant les
# cycles Skia/fontconfig/freetype/brotli sans recopier a la main le graphe de
# vcpkg dans Bouchaud.
VCPKG_ARCHIVES=$(find "$VCPKG/lib" -maxdepth 1 -type f -name '*.a' -print | sort)

info "== lien du temoin M6 (static-pie) =="
$CXX $CXXFLAGS "$RACINE/tools/ladybird/libgfx-probe.cpp" \
    -o "$SORTIE/libgfx-probe" \
    -Wl,--start-group \
    "$SORTIE/libGfx.a" \
    "$COMP/libCompress.a" \
    "$IPC/libIPC.a" \
    "$JS/libJS.a" \
    "$GC/libGC.a" \
    "$UNI/libUnicode.a" \
    "$CRYPTO/libCryptoMin.a" \
    "$CORE/libCore.a" \
    "$AK/libAK.a" \
    "$RUSTGFX/liblibgfx_rust.a" \
    "$RUST/liblibunicode_rust.a" \
    "$RUST/liblibregex_rust.a" \
    "$RUST/libliburl_rust.a" \
    "$RUST/liblibjs_rust.a" \
    "$RUST/liblibtextcodec_rust.a" \
    $VCPKG_ARCHIVES \
    -Wl,--end-group \
    -Wl,--allow-multiple-definition \
    -L"$DEPS/lib" -lsimdjson -ltommath -lpsl -lfmt -lsimdutf -lmimalloc \
    -L"$ICU/lib" -licui18n -licuuc -licudata \
    -lpthread -lm -ldl -lrt -lutil

[ -x "$SORTIE/libgfx-probe" ] || { rouge "libgfx-probe absent"; exit 1; }

vert "libgfx-probe pret"
file "$SORTIE/libgfx-probe" | sed 's/^/  /'

if [ "$CIBLE" = "hote" ]; then
    echo
    "$SORTIE/libgfx-probe"
fi
