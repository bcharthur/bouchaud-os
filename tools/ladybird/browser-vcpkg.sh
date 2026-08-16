#!/bin/bash
# Dependances du navigateur Ladybird complet, sans backend GPU.
# Reutilise le clone vcpkg M6 et son cache de telechargements.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$(pwd)
BASELINE=40f3c709db80acf154ac4b17a1f83c564ebd022e
VCPKG="$ROOT/third_party/vcpkg-gfx"
MANIFEST="$ROOT/third_party/vcpkg-browser-manifest"
INSTALLED="$ROOT/third_party/vcpkg-browser-installed"
DOWNLOADS="$ROOT/third_party/vcpkg-downloads"
OVERLAY_PORTS="$ROOT/third_party/ladybird/Meta/CMake/vcpkg/overlay-ports"

say(){ printf '\033[1;36m%s\033[0m\n' "$*"; }
ok(){ printf '\033[32m%s\033[0m\n' "$*"; }

if [ ! -x "$VCPKG/vcpkg" ]; then
    say "vcpkg M6 absent : amorcage"
    ./tools/ladybird/build-vcpkg-gfx.sh
fi

# M6 amorce volontairement vcpkg avec un clone shallow (`--depth 1`) pour
# construire seulement Skia. Le navigateur complet utilise en revanche le
# mode manifeste/versioning de vcpkg : le registre builtin reference des arbres
# Git historiques (dbus, libwebp, SDL3, ...). Un depot shallow peut connaitre
# le commit baseline tout en ne possedant pas ces objets, ce qui produit :
#   fatal: failed to unpack tree object ...
# Avant le premier build navigateur, on transforme donc ce clone en depot
# complet. Cette operation n'arrive qu'une fois : le cache GitHub conserve
# ensuite le vcpkg non-shallow pour les runs suivants.
if [ "$(git -C "$VCPKG" rev-parse --is-shallow-repository)" = "true" ]; then
    say "vcpkg : conversion du clone shallow en historique complet"
    git -C "$VCPKG" fetch -q --unshallow origin
else
    git -C "$VCPKG" fetch -q origin
fi

git -C "$VCPKG" fetch -q origin "$BASELINE"
git -C "$VCPKG" checkout -q --detach "$BASELINE"
mkdir -p "$MANIFEST" "$INSTALLED" "$DOWNLOADS"

# Start from Ladybird's own manifest, including its exact overrides, then remove
# only the GPU backend that Bouchaud deliberately does not expose at M6-M8.
# Platform expressions are left untouched: vcpkg will naturally skip Qt/Win/macOS
# dependencies on this Linux builder.
./tools/ladybird/fetch.sh
python3 - "$ROOT/third_party/ladybird/vcpkg.json" "$MANIFEST/vcpkg.json" <<'PYMANIFEST'
import json, sys
src, dst = sys.argv[1:]
with open(src, encoding="utf-8") as f:
    data = json.load(f)

remove = {"vulkan", "vulkan-headers", "vulkan-memory-allocator"}
out = []
for dep in data.get("dependencies", []):
    name = dep if isinstance(dep, str) else dep.get("name")
    if name in remove:
        continue
    if isinstance(dep, dict) and name == "skia":
        dep = dict(dep)
        dep["features"] = [x for x in dep.get("features", []) if x != "vulkan"]
    out.append(dep)
data["dependencies"] = out
# Keep builtin-baseline and *all* upstream overrides exactly as pinned.
with open(dst, "w", encoding="utf-8") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
PYMANIFEST

# Ladybird carries project-specific vcpkg ports that are not present in the
# builtin registry (notably pdfjs and wuffs, and pinned variants such as
# simdutf/angle).  The manifest alone is therefore insufficient: use the exact
# overlay directory from the same pinned Ladybird source tree.
[ -d "$OVERLAY_PORTS/pdfjs" ] || { echo "overlay Ladybird pdfjs absent: $OVERLAY_PORTS" >&2; exit 1; }
[ -d "$OVERLAY_PORTS/wuffs" ] || { echo "overlay Ladybird wuffs absent: $OVERLAY_PORTS" >&2; exit 1; }

say "== vcpkg navigateur Ladybird =="
printf '  overlay ports : %s\n' "$OVERLAY_PORTS"
export VCPKG_DOWNLOADS="$DOWNLOADS"
"$VCPKG/vcpkg" install \
    --x-manifest-root="$MANIFEST" \
    --x-install-root="$INSTALLED" \
    --overlay-ports="$OVERLAY_PORTS" \
    --triplet x64-linux \
    --clean-after-build

PREFIX="$INSTALLED/x64-linux"
for f in "$PREFIX/lib/libskia.a" "$PREFIX/lib/libssl.a" "$PREFIX/lib/libcrypto.a"; do
    [ -f "$f" ] || { echo "archive attendue absente: $f" >&2; exit 1; }
done
ok "dependances navigateur pretes : $PREFIX"
printf '  archives : %s\n' "$(find "$PREFIX/lib" -maxdepth 1 -name '*.a' | wc -l)"
printf '  taille   : %s\n' "$(du -sh "$PREFIX" | cut -f1)"
