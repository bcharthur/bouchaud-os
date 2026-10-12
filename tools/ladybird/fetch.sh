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
    # P19_WSL_MANIFEST_CRLF_V1: CRLF from Windows must not become part of the Git refspec.
    grep -m1 -E '^\s+sha\s+[0-9a-f]{40}' "$MANIFESTE" | awk '{print $2}' | tr -d '\r'
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
# P19_WSL_MANIFEST_SHA_GUARD_V1: fail before contacting Git if the SHA is contaminated.
if ! [[ "$SHA" =~ ^[0-9a-f]{40}$ ]]; then
    rouge "SHA Ladybird invalide: 40 caracteres hexadecimaux attendus (CRLF / argument)."
    exit 1
fi

# P19_PORTABLE_PATCHES_V1: Normalize patch input (not the tracked .patch or
# pinned upstream source), refuse an incompatible patch with the REAL error.
# A retry at the correct SHA must verify/apply missing patches, never exit early.
applique_patches_bouchaud() {
    local mode="$1"
    if [ ! -d "$PATCHES" ] || ! ls "$PATCHES"/*.patch >/dev/null 2>&1; then
        info "aucun patch Bouchaud (arbre upstream pur)"
        return 0
    fi
    local tmpdir patchname normalise p tmpdir_quoted
    tmpdir=$(mktemp -d)
    # Expand the trap path at registration: Bash function locals disappear
    # before an EXIT trap when this function returns an error under set -e.
    printf -v tmpdir_quoted '%q' "$tmpdir"
    trap "rm -rf -- $tmpdir_quoted" EXIT
    for p in "$PATCHES"/*.patch; do
        patchname=$(basename "$p")
        normalise="$tmpdir/$patchname"
        # `*.patch` is not covered by .gitattributes eol=lf on Windows.
        # GNU sed removes only an end-of-line CR; no patch hunk is edited.
        sed 's/$//' "$p" > "$normalise"
        if [ ! -s "$normalise" ]; then
            rouge "P19_PATCH_INVALID vide=$patchname"
            return 1
        fi
        if git -C "$CIBLE" apply --reverse --check "$normalise" >/dev/null 2>&1; then
            info "P19_PATCH_VERIFIED already-applied=$patchname"
            continue
        fi
        if [ "$mode" = verifier ]; then
            rouge "P19_PATCH_MISSING_OR_DIVERGENT=$patchname"
            git -C "$CIBLE" apply --check "$normalise" || true
            return 1
        fi
        info "P19_PATCH_CHECK_START=$patchname"
        if ! git -C "$CIBLE" apply --check "$normalise"; then
            rouge "P19_PATCH_INCOMPATIBLE=$patchname (source ou contexte modifie)"
            return 1
        fi
        git -C "$CIBLE" apply "$normalise" || return 1
        if ! git -C "$CIBLE" apply --reverse --check "$normalise"; then
            rouge "P19_PATCH_POST_APPLY_CHECK_FAILED=$patchname"
            return 1
        fi
        info "P19_PATCH_APPLIED_OK=$patchname"
    done
    rm -rf "$tmpdir"
    trap - EXIT
}

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
    applique_patches_bouchaud verifier
    vert "Ladybird au SHA epingle et patches verifies : $SHA"
    exit 0
fi

info "== Ladybird $SHA =="

if [ -e "$CIBLE/.git" ]; then
    ACTUEL=$(git -C "$CIBLE" rev-parse HEAD 2>/dev/null || echo "")
    if [ "$ACTUEL" = "$SHA" ]; then
        vert "deja au bon SHA, verification des patches Bouchaud"
    else
        info "arbre present a $ACTUEL, mise a jour"
        info "P19_FETCH_PROGRESS_V1 stage=fetch-start"
        git -C "$CIBLE" fetch --progress --depth 1 origin "$SHA"
        info "P19_FETCH_PROGRESS_V1 stage=fetch-complete"
        info "P19_FETCH_PROGRESS_V1 stage=checkout-start"
        git -C "$CIBLE" checkout --progress --force --detach FETCH_HEAD
        info "P19_FETCH_PROGRESS_V1 stage=checkout-complete"
    fi
else
    info "clone superficiel"
    rm -rf "$CIBLE"
    mkdir -p "$(dirname "$CIBLE")"
    git init -q "$CIBLE"
    git -C "$CIBLE" remote add origin "$DEPOT"
    info "P19_FETCH_PROGRESS_V1 stage=fetch-start"
    git -C "$CIBLE" fetch --progress --depth 1 origin "$SHA"
    info "P19_FETCH_PROGRESS_V1 stage=fetch-complete"
    info "P19_FETCH_PROGRESS_V1 stage=checkout-start"
    git -C "$CIBLE" checkout --progress --force --detach FETCH_HEAD
    info "P19_FETCH_PROGRESS_V1 stage=checkout-complete"
fi

OBTENU=$(git -C "$CIBLE" rev-parse HEAD)
if [ "$OBTENU" != "$SHA" ]; then
    rouge "le depot a rendu $OBTENU au lieu de $SHA"
    exit 1
fi

# Les divergences vivent en patches numerotes, jamais en modifications directes
# de l'arbre : c'est ce qui permet de dire a tout moment ce que nous avons change
# par rapport a upstream, et de proposer en amont ce qui le merite.
applique_patches_bouchaud installer

vert "Ladybird pret dans $CIBLE au SHA $OBTENU"
