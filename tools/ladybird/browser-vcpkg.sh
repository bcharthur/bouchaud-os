#!/bin/bash
# Dependances du navigateur Ladybird complet, sans backend GPU.
# Reutilise le clone vcpkg M6 et son cache de telechargements/binaire.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$(pwd)
BASELINE=40f3c709db80acf154ac4b17a1f83c564ebd022e
VCPKG="$ROOT/third_party/vcpkg-gfx"
MANIFEST="$ROOT/third_party/vcpkg-browser-manifest"
INSTALLED="$ROOT/third_party/vcpkg-browser-installed"
DOWNLOADS="$ROOT/third_party/vcpkg-downloads"
OVERLAY_PORTS="$ROOT/third_party/ladybird/Meta/CMake/vcpkg/overlay-ports"
LOCAL_OVERLAY_PORTS="$MANIFEST/overlay-ports"

# Ladybird epingle fontconfig 2.17.1#1. Son port vcpkg telecharge normalement
# une archive generee par gitlab.freedesktop.org. Cet endpoint a deja bloque
# plusieurs executions GitHub Actions (504), alors que le depot Git lui-meme est
# miroir sur GitHub. On conserve EXACTEMENT le port vcpkg #1 et ses patches,
# mais on remplace seulement son transport de source par un commit immuable du
# miroir. Le tag 2.17.1 du miroir pointe sur ce commit.
FONTCONFIG_PORT_TREE=91f5aca0263a76ef0580e8250dcb01c0132f499e
FONTCONFIG_GIT_COMMIT=6d0a98982ec351c165c9224c8b7dbdfca3010e47
FONTCONFIG_FETCH_REF=2.17.1

say(){ printf '\033[1;36m%s\033[0m\n' "$*"; }
ok(){ printf '\033[32m%s\033[0m\n' "$*"; }

if [ ! -x "$VCPKG/vcpkg" ]; then
    say "vcpkg M6 absent : amorcage"
    ./tools/ladybird/build-vcpkg-gfx.sh
fi

# M6 amorce volontairement vcpkg avec un clone shallow (`--depth 1`) pour
# construire seulement Skia. Le navigateur complet utilise en revanche le
# mode manifeste/versioning de vcpkg : le registre builtin reference des arbres
# Git historiques. Un depot shallow peut connaitre le commit baseline tout en
# ne possedant pas ces objets.
if [ "$(git -C "$VCPKG" rev-parse --is-shallow-repository)" = "true" ]; then
    say "vcpkg : conversion du clone shallow en historique complet"
    git -C "$VCPKG" fetch -q --unshallow origin
else
    git -C "$VCPKG" fetch -q origin
fi

git -C "$VCPKG" fetch -q origin "$BASELINE"
git -C "$VCPKG" checkout -q --detach "$BASELINE"
mkdir -p "$MANIFEST" "$DOWNLOADS"

./tools/ladybird/fetch.sh
python3 - "$ROOT/third_party/ladybird/vcpkg.json" "$MANIFEST/vcpkg.json" <<'PYMANIFEST'
import json, sys
src, dst = sys.argv[1:]
with open(src, encoding="utf-8") as f:
    data = json.load(f)

# Bouchaud n'expose aucun GPU et ne fournit ni bus de session D-Bus ni service
# de decouverte de proxy du bureau Linux. Ces dependances d'integration desktop
# ne font pas partie du WebContent natif Bouchaud.
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

with open(dst, "w", encoding="utf-8") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
PYMANIFEST

[ -d "$OVERLAY_PORTS/pdfjs" ] || { echo "overlay Ladybird pdfjs absent: $OVERLAY_PORTS" >&2; exit 1; }
[ -d "$OVERLAY_PORTS/wuffs" ] || { echo "overlay Ladybird wuffs absent: $OVERLAY_PORTS" >&2; exit 1; }

# --- fontconfig : meme port, source Git robuste ------------------------------
#
# Le run #19 a demontre que le port compile ; son unique panne est le 504 de
# l'archive GitLab. Le baseline vcpkg possede encore l'arbre exact du port #1.
# On l'extrait comme overlay local, puis on ne remplace que vcpkg_from_gitlab
# par vcpkg_from_git vers le miroir GitHub epingle. Les patches, options Meson,
# dependances et numero de port restent donc ceux demandes par Ladybird.
FONTCONFIG_OVERLAY="$LOCAL_OVERLAY_PORTS/fontconfig"
rm -rf "$FONTCONFIG_OVERLAY"
mkdir -p "$FONTCONFIG_OVERLAY"
git -C "$VCPKG" archive "$FONTCONFIG_PORT_TREE" | tar -x -C "$FONTCONFIG_OVERLAY"

python3 - "$FONTCONFIG_OVERLAY/portfile.cmake" "$FONTCONFIG_GIT_COMMIT" "$FONTCONFIG_FETCH_REF" <<'PYFONTCONFIG'
from pathlib import Path
import sys

path = Path(sys.argv[1])
commit = sys.argv[2]
fetch_ref = sys.argv[3]
text = path.read_text(encoding="utf-8")
start = text.find("vcpkg_from_gitlab(")
patches = text.find("    PATCHES", start)
if start < 0 or patches < 0:
    raise SystemExit("port fontconfig inattendu : bloc vcpkg_from_gitlab introuvable")
replacement = f'''vcpkg_from_git(\n    OUT_SOURCE_PATH SOURCE_PATH\n    URL https://github.com/arthenica/fontconfig.git\n    REF {commit}\n    FETCH_REF {fetch_ref}\n'''
text = text[:start] + replacement + text[patches:]
path.write_text(text, encoding="utf-8")
PYFONTCONFIG

grep -Fq "REF $FONTCONFIG_GIT_COMMIT" "$FONTCONFIG_OVERLAY/portfile.cmake"
grep -Fq '"port-version": 1' "$FONTCONFIG_OVERLAY/vcpkg.json"
say "fontconfig : port 2.17.1#1 conserve, source miroir GitHub epinglee"

# IMPORTANT : l'install root n'est PAS un cache de compilation. Il contient
# l'etat de resolution du manifeste. Le restaurer depuis une ancienne tentative
# peut conserver des dependances qui ont depuis ete retirees du manifeste.
# C'est exactement ce qui a maintenu `dbus[systemd]` vivant apres sa suppression
# du manifeste Bouchaud.
#
# On reconstruit donc cet arbre a chaque run. Le travail couteux n'est pas perdu :
# vcpkg restaure les paquets deja construits depuis son binary cache
# (~/.cache/vcpkg/archives) et reutilise les sources dans $DOWNLOADS.
say "vcpkg : reconstruction propre de l'install root navigateur"
rm -rf "$INSTALLED"
mkdir -p "$INSTALLED"

# Garde-fou contre une regression du transformateur de manifeste lui-meme.
python3 - "$MANIFEST/vcpkg.json" <<'PYCHECK'
import json, sys
with open(sys.argv[1], encoding="utf-8") as f:
    data = json.load(f)
names = {d if isinstance(d, str) else d.get("name") for d in data.get("dependencies", [])}
for forbidden in ("dbus", "libproxy", "vulkan", "vulkan-headers", "vulkan-memory-allocator"):
    if forbidden in names:
        raise SystemExit(f"dependance interdite encore presente dans le manifeste: {forbidden}")
print("  manifeste Bouchaud propre : dbus/libproxy/vulkan absents")
PYCHECK

say "== vcpkg navigateur Ladybird =="
printf '  overlay ports Ladybird : %s\n' "$OVERLAY_PORTS"
printf '  overlay ports Bouchaud  : %s\n' "$LOCAL_OVERLAY_PORTS"
export VCPKG_DOWNLOADS="$DOWNLOADS"

# Reprise pour les pannes reseau transitoires. Comme l'install root est propre,
# une seconde tentative ne peut pas ressusciter le graphe d'un ancien manifeste.
#
# Ne PAS utiliser --clean-after-build ici : cette option supprime aussi les
# archives de $VCPKG_DOWNLOADS apres chaque paquet. Cela annulait exactement le
# cache de sources que ce script et le workflow cherchent a conserver. On garde
# le nettoyage des buildtrees et packages temporaires pour contenir l'espace
# disque, mais les telechargements verifies restent disponibles au run suivant.
ATTENTE=15
TENTATIVE=1
MAX=3
while :; do
    if "$VCPKG/vcpkg" install \
        --x-manifest-root="$MANIFEST" \
        --x-install-root="$INSTALLED" \
        --overlay-ports="$LOCAL_OVERLAY_PORTS" \
        --overlay-ports="$OVERLAY_PORTS" \
        --triplet x64-linux \
        --clean-buildtrees-after-build \
        --clean-packages-after-build
    then
        break
    fi
    find "$DOWNLOADS" -maxdepth 1 -name '*.part' -delete 2>/dev/null || true
    if [ "$TENTATIVE" -ge "$MAX" ]; then
        echo "vcpkg a echoue $MAX fois — ce n'est plus une panne passagere" >&2
        echo "  binary cache et archives restent disponibles pour le prochain run" >&2
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
