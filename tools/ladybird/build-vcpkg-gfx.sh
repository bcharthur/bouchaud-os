#!/bin/bash
# Dependances graphiques du port Ladybird pour Bouchaud OS.
#
# Le but n'est PAS de construire un Ladybird Linux puis de le copier dans
# Bouchaud. On utilise vcpkg uniquement comme constructeur de bibliotheques
# statiques correspondant au graphe upstream, puis ces archives sont liees dans
# nos ELF static-pie.
#
# Skia est volontairement CPU-only :
#   - pas de Vulkan
#   - pas d'OpenGL
#   - pas de Direct3D / Metal
#   - Freetype + Fontconfig pour le font manager Unix
#
# HarfBuzz est construit separement sans ICU afin de ne pas introduire une
# deuxieme version d'ICU dans l'ELF. LibUnicode continue d'utiliser l'ICU deja
# construite par tools/ladybird/build-icu.sh.

set -eu
cd "$(dirname "$0")/../.."
RACINE=$(pwd)

VCPKG_COMMIT=40f3c709db80acf154ac4b17a1f83c564ebd022e
VCPKG="$RACINE/third_party/vcpkg-gfx"
MANIFESTE="$RACINE/third_party/vcpkg-gfx-manifest"
INSTALLE="$RACINE/third_party/vcpkg-gfx-installed"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

mkdir -p "$RACINE/third_party"

if [ ! -d "$VCPKG/.git" ]; then
    info "== vcpkg : recuperation du baseline Ladybird =="
    git init -q "$VCPKG"
    git -C "$VCPKG" remote add origin https://github.com/microsoft/vcpkg.git
    git -C "$VCPKG" fetch --depth 1 origin "$VCPKG_COMMIT"
    git -C "$VCPKG" checkout --force --detach FETCH_HEAD
else
    ACTUEL=$(git -C "$VCPKG" rev-parse HEAD)
    if [ "$ACTUEL" != "$VCPKG_COMMIT" ]; then
        info "vcpkg : $ACTUEL -> $VCPKG_COMMIT"
        git -C "$VCPKG" fetch --depth 1 origin "$VCPKG_COMMIT"
        git -C "$VCPKG" checkout --force --detach FETCH_HEAD
    fi
fi

mkdir -p "$MANIFESTE"
cat > "$MANIFESTE/vcpkg.json" <<EOF
{
  "name": "bouchaud-ladybird-gfx",
  "version-string": "0.1.0",
  "builtin-baseline": "$VCPKG_COMMIT",
  "dependencies": [
    "brotli",
    "fontconfig",
    {
      "name": "harfbuzz",
      "default-features": false
    },
    "libjpeg-turbo",
    "libpng",
    {
      "name": "skia",
      "default-features": false,
      "features": [
        "freetype",
        "fontconfig"
      ]
    },
    "woff2",
    "zlib"
  ]
}
EOF

if [ ! -x "$VCPKG/vcpkg" ]; then
    info "== vcpkg : bootstrap =="
    "$VCPKG/bootstrap-vcpkg.sh" -disableMetrics
fi

info "== dependances graphiques Ladybird / Skia CPU =="
"$VCPKG/vcpkg" install \
    --triplet x64-linux \
    --x-manifest-root="$MANIFESTE" \
    --x-install-root="$INSTALLE" \
    --clean-after-build \
    --disable-metrics

[ -f "$INSTALLE/x64-linux/lib/libskia.a" ] || {
    rouge "Skia statique absent apres vcpkg"
    find "$INSTALLE" -maxdepth 4 -type f -name '*skia*' -print 2>/dev/null || true
    exit 1
}

PREFIXE="$INSTALLE/x64-linux"
vert "Skia CPU pret : $PREFIXE/lib/libskia.a"
printf '  archives : %s\n' "$(find "$PREFIXE/lib" -maxdepth 1 -name '*.a' | wc -l)"
printf '  taille   : %s\n' "$(du -sh "$PREFIXE" | cut -f1)"
