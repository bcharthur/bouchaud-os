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

# --- Ou vcpkg depose ses archives telechargees ------------------------------
#
# Par defaut, vcpkg les met dans `$VCPKG/downloads`, c'est-a-dire **dans** son
# propre depot git. On l'en sort : ce repertoire est le seul dont le contenu
# merite d'etre conserve entre deux executions, et le melanger a l'arbre vcpkg
# oblige a tout mettre en cache ensemble ou rien.
#
# Chaque archive est validee par son empreinte SHA512 avant usage. Un cache
# corrompu ou partiel ne peut donc pas empoisonner la construction : vcpkg
# retelecharge ce qui ne correspond pas.
export VCPKG_DOWNLOADS="${VCPKG_DOWNLOADS:-$RACINE/third_party/vcpkg-downloads}"
mkdir -p "$VCPKG_DOWNLOADS"

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

# --- L'installation, avec reprise ------------------------------------------
#
# vcpkg va chercher chaque paquet chez son editeur : sourceware pour bzip2,
# gitlab.freedesktop.org pour freetype, et une quinzaine d'autres hotes. Aucun
# n'est sous notre controle, et il suffit qu'un seul reponde 504 pour que la
# construction entiere s'arrete — c'est exactement ce qui est arrive avec
# freetype.
#
# On reessaie donc, avec une attente croissante. Ce n'est pas une precaution
# vague : vcpkg est **reprenable**. Chaque paquet deja construit est reconnu et
# saute, et chaque archive deja telechargee reste dans `$VCPKG_DOWNLOADS`. Un
# second essai repart donc de la ou le premier s'est arrete, et ne repaie ni les
# telechargements ni les compilations acquis.
#
# Les trois tentatives internes de vcpkg ne suffisent pas : elles s'enchainent
# en quelques secondes, ce qui ne laisse pas le temps a une panne passagere de
# se resorber.
info "== dependances graphiques Ladybird / Skia CPU =="
ATTENTE=15
TENTATIVE=1
MAX=4
while :; do
    if "$VCPKG/vcpkg" install \
        --triplet x64-linux \
        --x-manifest-root="$MANIFESTE" \
        --x-install-root="$INSTALLE" \
        --clean-after-build \
        --disable-metrics
    then
        break
    fi
    # Les fragments d'un telechargement interrompu (`*.part`) ne servent a
    # rien — vcpkg reprend du debut — et gonfleraient le cache sans profit.
    #
    # Ce menage vient **avant** le test de la derniere tentative, et non apres.
    # Place ensuite, il etait saute precisement dans le cas ou il compte : la
    # sortie en erreur, suivie du `if: always()` du workflow qui enregistre le
    # repertoire. Un gros telechargement coupe au dernier essai aurait donc ete
    # mis en cache, restaure aux executions suivantes, et aurait consomme le
    # quota pour un fragment inutilisable — l'inverse de ce que la boucle
    # cherche a preserver.
    find "$VCPKG_DOWNLOADS" -maxdepth 1 -name '*.part' -delete 2>/dev/null || true
    if [ "$TENTATIVE" -ge "$MAX" ]; then
        rouge "vcpkg a echoue $MAX fois — ce n'est plus une panne passagere"
        rouge "  les archives deja obtenues restent dans $VCPKG_DOWNLOADS"
        exit 1
    fi
    info "  tentative $TENTATIVE/$MAX echouee ; reprise dans ${ATTENTE}s"
    sleep "$ATTENTE"
    ATTENTE=$((ATTENTE * 2))
    TENTATIVE=$((TENTATIVE + 1))
done

[ -f "$INSTALLE/x64-linux/lib/libskia.a" ] || {
    rouge "Skia statique absent apres vcpkg"
    find "$INSTALLE" -maxdepth 4 -type f -name '*skia*' -print 2>/dev/null || true
    exit 1
}

PREFIXE="$INSTALLE/x64-linux"
vert "Skia CPU pret : $PREFIXE/lib/libskia.a"
printf '  archives : %s\n' "$(find "$PREFIXE/lib" -maxdepth 1 -name '*.a' | wc -l)"
printf '  taille   : %s\n' "$(du -sh "$PREFIXE" | cut -f1)"
