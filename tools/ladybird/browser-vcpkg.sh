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

# Ce que Bouchaud retire du manifeste d'upstream, et pourquoi.
#
# `vulkan*` : Bouchaud n'expose aucun GPU. Skia tourne en CPU, la feature
# `vulkan` de skia est retiree plus bas.
#
# `dbus` et `libproxy` : integration de bureau Linux — bus de session pour les
# notifications et les portails, et decouverte du proxy systeme. Bouchaud n'a
# ni l'un ni l'autre. Ce ne sont pas des suppositions : dans l'arbre epingle,
# **aucun CMakeLists ni aucune source de Ladybird ne les reference**. Le seul
# « dbus » du depot est une regle d'installation qui depose un fichier
# `.service` dans `share/dbus-1/services`, un artefact d'empaquetage de
# l'application de bureau, pas une dependance de construction. WebContent ne
# les lie pas.
#
# Ils ne sont pas retires par confort : `dbus` est ce qui fait echouer la
# construction. Son archive vient de `gitlab.freedesktop.org` — le meme hote
# qui avait rendu 504 pour freetype en M6 — et cet hote rend desormais 403 aux
# runners GitHub :
#
#     error: curl operation failed with response code 403.
#     error: Not a transient network error, won't retry download from
#            https://gitlab.freedesktop.org//dbus/dbus/-/archive/dbus-1.16.2/...
#     error: building dbus:x64-linux failed with: BUILD_FAILED
#
# vcpkg classe 403 comme **non transitoire** et ne reessaie pas : aucune boucle
# de reprise ne peut rattraper cela. La seule issue correcte est de ne pas
# demander un paquet dont on n'a pas besoin. `libproxy` part avec, parce qu'il
# est lui aussi inutilise et qu'il tire `dbus`.
remove = {
    "vulkan", "vulkan-headers", "vulkan-memory-allocator",
    "dbus", "libproxy",
}
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

# Reprise, comme pour Skia en M6. vcpkg est reprenable : un paquet deja
# construit est reconnu et saute, une archive deja obtenue reste dans
# `$VCPKG_DOWNLOADS`. Un second essai repart donc d'ou le premier s'est arrete.
#
# A ne pas confondre avec le cas `dbus` : une erreur que vcpkg declare **non
# transitoire** (403, empreinte invalide) se reproduira a l'identique, et la
# boucle ne fait alors que la constater trois fois plus vite. Elle est la pour
# les 504 et les coupures — on en a deja vu un sur ce meme hote.
ATTENTE=15
TENTATIVE=1
MAX=3
while :; do
    if "$VCPKG/vcpkg" install \
        --x-manifest-root="$MANIFEST" \
        --x-install-root="$INSTALLED" \
        --overlay-ports="$OVERLAY_PORTS" \
        --triplet x64-linux \
        --clean-after-build
    then
        break
    fi
    find "$DOWNLOADS" -maxdepth 1 -name '*.part' -delete 2>/dev/null || true
    if [ "$TENTATIVE" -ge "$MAX" ]; then
        echo "vcpkg a echoue $MAX fois — ce n'est plus une panne passagere" >&2
        echo "  les archives obtenues restent dans $DOWNLOADS" >&2
        exit 1
    fi
    say "  tentative $TENTATIVE/$MAX echouee ; reprise dans ${ATTENTE}s"
    sleep "$ATTENTE"
    ATTENTE=$((ATTENTE * 2))
    TENTATIVE=$((TENTATIVE + 1))
done

PREFIX="$INSTALLED/x64-linux"
for f in "$PREFIX/lib/libskia.a" "$PREFIX/lib/libssl.a" "$PREFIX/lib/libcrypto.a"; do
    [ -f "$f" ] || { echo "archive attendue absente: $f" >&2; exit 1; }
done
ok "dependances navigateur pretes : $PREFIX"
printf '  archives : %s\n' "$(find "$PREFIX/lib" -maxdepth 1 -name '*.a' | wc -l)"
printf '  taille   : %s\n' "$(du -sh "$PREFIX" | cut -f1)"
