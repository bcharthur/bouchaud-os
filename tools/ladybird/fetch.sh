#!/bin/bash
# Recupere l'arbre Ladybird au SHA epingle, applique nos patches.
#
#   ./tools/ladybird/fetch.sh            recupere au SHA de third_party/UPSTREAM.md
#   ./tools/ladybird/fetch.sh --verifie  verifie seulement, ne modifie rien
#   ./tools/ladybird/fetch.sh <sha>      recupere un SHA precis (essai de montee)
#
# ## Pourquoi un script plutot qu'un submodule
#
# La conception initiale (docs/ladybird/DEPENDENCIES.md) retenait un submodule.
# En l'implementant, le defaut est apparu : le SHA aurait alors ete inscrit a
# deux endroits — dans l'index git et dans `third_party/UPSTREAM.md` — et il
# aurait fallu une regle pour dire lequel gagne en cas de desaccord. Deux
# sources de verite pour une meme valeur, c'est precisement ce qu'on reproche
# ailleurs dans ce depot.
#
# Le SHA vit donc a un seul endroit, lisible et revisable dans un diff, et ce
# script en est l'unique consommateur. Il n'a besoin que de `git`, fonctionne
# depuis une archive du depot comme depuis un clone, et ne demande pas de
# `--recursive` que personne ne tape.
#
# ## Ce qu'il ne fait pas
#
# Il ne construit rien. La construction est le travail des scripts de portage,
# qui supposent l'arbre deja la.

set -eu
cd "$(dirname "$0")/../.."
RACINE=$(pwd)

DEPOT=https://github.com/LadybirdBrowser/ladybird
CIBLE=third_party/ladybird
MANIFESTE=third_party/UPSTREAM.md
PATCHES=tools/ladybird/patches

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

# Le SHA epingle se lit dans le manifeste, pas dans le submodule : le manifeste
# est ce qu'un humain a ecrit, le submodule ce que git a enregistre. Quand les
# deux divergent, c'est le manifeste qui dit l'intention.
sha_epingle() {
    grep -m1 -E '^\s+sha\s+[0-9a-f]{40}' "$MANIFESTE" | awk '{print $2}'
}

VERIFIE=0
SHA_DEMANDE=""
case "${1:-}" in
    --verifie) VERIFIE=1 ;;
    "") ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) SHA_DEMANDE="$1" ;;
esac

SHA=${SHA_DEMANDE:-$(sha_epingle)}
if [ -z "$SHA" ]; then
    rouge "aucun SHA dans $MANIFESTE"
    exit 1
fi

if [ "$VERIFIE" = 1 ]; then
    if [ ! -e "$CIBLE/.git" ]; then
        rouge "$CIBLE absent — lancer $0 sans argument"
        exit 1
    fi
    ACTUEL=$(git -C "$CIBLE" rev-parse HEAD)
    if [ "$ACTUEL" != "$SHA" ]; then
        rouge "SHA discordant"
        echo "  attendu (manifeste) : $SHA"
        echo "  present  (arbre)    : $ACTUEL"
        exit 1
    fi
    vert "Ladybird au SHA epingle : $SHA"
    exit 0
fi

info "== Ladybird $SHA =="

if [ -e "$CIBLE/.git" ]; then
    ACTUEL=$(git -C "$CIBLE" rev-parse HEAD 2>/dev/null || echo "")
    if [ "$ACTUEL" = "$SHA" ]; then
        vert "deja au bon SHA, rien a faire"
        exit 0
    fi
    info "arbre present a $ACTUEL, mise a jour"
    git -C "$CIBLE" fetch --depth 1 origin "$SHA"
    git -C "$CIBLE" checkout --force --detach FETCH_HEAD
else
    info "clone superficiel"
    rm -rf "$CIBLE"
    mkdir -p "$(dirname "$CIBLE")"
    git init -q "$CIBLE"
    git -C "$CIBLE" remote add origin "$DEPOT"
    git -C "$CIBLE" fetch --depth 1 origin "$SHA"
    git -C "$CIBLE" checkout --force --detach FETCH_HEAD
fi

OBTENU=$(git -C "$CIBLE" rev-parse HEAD)
if [ "$OBTENU" != "$SHA" ]; then
    rouge "le depot a rendu $OBTENU au lieu de $SHA"
    exit 1
fi

# Les divergences vivent en patches numerotes, jamais en modifications directes
# de l'arbre : c'est ce qui permet de dire a tout moment ce que nous avons change
# par rapport a upstream, et de proposer en amont ce qui le merite.
if [ -d "$PATCHES" ] && ls "$PATCHES"/*.patch >/dev/null 2>&1; then
    info "application des patches Bouchaud"
    for p in "$PATCHES"/*.patch; do
        echo "  PATCH $(basename "$p")"
        git -C "$CIBLE" apply --check "$RACINE/$p" 2>/dev/null || {
            rouge "  le patch ne s'applique pas a ce SHA : $(basename "$p")"
            rouge "  c'est le signal d'une divergence upstream a examiner, pas a forcer"
            exit 1
        }
        git -C "$CIBLE" apply "$RACINE/$p"
    done
else
    info "aucun patch Bouchaud (l'arbre est upstream pur)"
fi

vert "Ladybird pret dans $CIBLE au SHA $OBTENU"
