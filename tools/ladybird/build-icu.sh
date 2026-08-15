#!/bin/bash
# Construit ICU en statique, la dependance qui bloquait LibUnicode.
#
#   ./tools/ladybird/build-icu.sh          hote
#   ./tools/ladybird/build-icu.sh --cible  Bouchaud (objets PIC, sans dlopen)
#
# ## Pourquoi ne pas prendre l'ICU de la distribution
#
# Parce qu'il est trop vieux, et que ca ne se voit qu'au quatrieme fichier.
# Sur les 23 sources de LibUnicode, 19 compilent sans broncher contre l'ICU 74
# de Debian. Les 4 dernieres echouent sur des API que Ladybird emploie et que
# 74 n'expose pas encore :
#
#   - `LibUnicode/CharacterTypes.cpp` appelle `icu::UnicodeSet::strings()`, qui
#     est **privee** jusqu'a ICU 76 et publique a partir de 77 ;
#   - `Calendars/AdjustedEraCalendar.h` et `Calendars/ChineseDangiCalendar.h`
#     derivent d'`icu::Calendar`, dont les signatures virtuelles ont change :
#     leurs `override` ne recouvrent plus rien.
#
# Aucun de ces echecs ne se corrige de notre cote sans modifier du code
# Ladybird. C'est une question de version, et la reponse est de construire la
# bonne version.
#
# ## Quelle version
#
# `vcpkg.json` d'upstream epingle **ICU 78.3**. Or aucun tag `release-78-*`
# n'existe sur le depot public d'ICU : le plus recent y est `release-77-1`. La
# reference de vcpkg ne correspond donc a aucune etiquette amont accessible, et
# `release-77-1` — qui expose bien `strings()` en public et compile les quatre
# fichiers — devient la cible retenue. C'est un ecart assume par rapport a
# upstream, note dans `docs/ladybird/LIBJS_PORT.md`.
#
# ## Les deux options de configuration qui comptent
#
# `--disable-dyload` : sans lui, `putil.cpp` compile `uprv_dl_open()`, donc un
# appel a `dlopen`. Dans un binaire `-static-pie` destine a Bouchaud, cette
# reference lie une fonction qui ne peut pas fonctionner — l'editeur de liens
# le dit d'ailleurs lui-meme (« using 'dlopen' in statically linked
# applications requires at runtime the shared libraries from the glibc version
# used for linking »). Le chargement dynamique ne sert a rien ici : on le
# retire plutot que d'embarquer un chemin de code mort qui planterait s'il
# etait pris.
#
# Attention au piege : l'option qu'on croit chercher est `--disable-plugins`,
# et elle ne fait rien. Les greffons sont **deja** desactives par defaut
# (`plugins=false` dans configure), et pourtant `dlopen` reste : le garde-fou
# dans `putil.cpp` est `U_ENABLE_DYLOAD`, pilote par `--enable-dyload`, pas par
# les greffons. Passer `--disable-plugins` seul donne donc un binaire qui
# reference toujours `dlopen`, avec la satisfaction trompeuse d'avoir pose
# l'option. D'ou la verification `nm` a la fin de ce script : une option qui ne
# fait pas ce qu'on croit ne se voit nulle part ailleurs.
#
# `--with-data-packaging=static` : les donnees Unicode deviennent une archive
# (`libicudata.a`) au lieu d'un fichier a cote du binaire. Il n'y a pas de
# systeme de fichiers garanti a cote d'un binaire Bouchaud ; les donnees
# doivent etre dans le binaire.
#
# ## Le cout, dit franchement
#
# `libicudata.a` fait 31 Mio, en un seul objet — donc l'edition de liens le
# prend en entier ou pas du tout. C'est la vraie facture d'ICU, et elle est
# connue d'avance plutot que decouverte a la fin. Si elle devient inacceptable
# pour l'image, la reponse est `ICU_DATA_FILTER_FILE`, qui restreint le jeu de
# donnees construit. Ce n'est pas fait ici : on mesure d'abord ce dont LibJS a
# reellement besoin, on filtre ensuite.

set -eu
cd "$(dirname "$0")/../.."
RACINE=$(pwd)

TAG=release-77-1
CIBLE=hote
if [ "${1:-}" = "--cible" ]; then
    CIBLE=bouchaud
fi

PREFIXE="$RACINE/third_party/icu-$CIBLE"
CHANTIER="$RACINE/third_party/icu-src"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

if [ -f "$PREFIXE/lib/libicuuc.a" ] && [ "${FORCE:-0}" != "1" ]; then
    vert "ICU deja construit : $PREFIXE (FORCE=1 pour refaire)"
    exit 0
fi

# --- Hote et cible partagent les memes archives -----------------------------
#
# Ce n'est pas un raccourci, c'est une consequence de la chaine actuelle. Les
# autres scripts distinguent `hote` de `bouchaud` par `-static-pie -fPIE` ;
# or `-static-pie` est un drapeau d'**edition de liens**, sans effet sur un
# `.o`, et `-fPIE` est deja couvert par le `-fPIC` qu'ICU recoit dans les deux
# cas. Le compilateur, la libc et la version sont par ailleurs les memes. Les
# deux constructions produiraient donc des archives identiques, au prix d'une
# vingtaine de minutes de machine.
#
# On lie plutot le prefixe cible sur celui de l'hote. Le jour ou la cible
# recevra sa propre chaine croisee — une libc differente, un autre triplet —
# cette equivalence tombera, et c'est ici qu'il faudra la retirer. Le lien
# symbolique rend ce moment visible : il faudra le supprimer pour construire
# pour de vrai, plutot que decouvrir qu'on a teste l'hote en croyant tester la
# cible.
if [ "$CIBLE" = bouchaud ] && [ -f "$RACINE/third_party/icu-hote/lib/libicuuc.a" ]; then
    info "== ICU (cible) : meme chaine que l'hote, archives partagees =="
    ln -sfn "$RACINE/third_party/icu-hote" "$PREFIXE"
    vert "  ok    $PREFIXE -> icu-hote"
    exit 0
fi

info "== ICU $TAG ($CIBLE) =="

if [ ! -d "$CHANTIER/icu4c" ]; then
    info "  GIT   $TAG"
    rm -rf "$CHANTIER"
    # `--depth 1` sur l'etiquette : l'historique d'ICU pese des gigaoctets et
    # nous n'en avons aucun usage. L'etiquette est fixe, donc la recuperation
    # est reproductible malgre la profondeur 1.
    git clone --depth 1 --branch "$TAG" \
        https://github.com/unicode-org/icu.git "$CHANTIER"
fi

SRC="$CHANTIER/icu4c/source"

# Les objets doivent etre PIC dans les deux cas : la cible Bouchaud lie en
# `-static-pie`, et l'hote sert de banc d'essai a cette meme chaine. Un ICU
# construit sans PIC produirait un echec d'edition de liens qu'on ne
# decouvrirait qu'au moment de croiser.
export CFLAGS="${CFLAGS:-} -fPIC -O2"
export CXXFLAGS="${CXXFLAGS:-} -fPIC -O2"

info "  CONF  --prefix=$PREFIXE"
( cd "$SRC" && ./configure \
    --prefix="$PREFIXE" \
    --enable-static --disable-shared \
    --disable-dyload --disable-plugins \
    --disable-samples --disable-tests --disable-extras \
    --with-data-packaging=static \
    > "$CHANTIER/configure.log" 2>&1 ) || {
        rouge "configure a echoue — voir $CHANTIER/configure.log"
        tail -20 "$CHANTIER/configure.log"
        exit 1
    }

# `make clean` avant `make`, et ce n'est pas de la superstition : c'est le
# piege qui a effectivement fait echouer la verification `nm` la premiere fois.
#
# `configure` regenere `icudefs.mk` avec les nouveaux drapeaux — on y lit bien
# `-DU_ENABLE_DYLOAD=0`. Mais `make` ne compare que les dates des `.c` et des
# `.o`, et les sources n'ont pas bouge : les objets de la construction
# precedente sont conserves tels quels, avec les anciens drapeaux. On obtient
# alors une bibliotheque qui contredit sa propre configuration, et rien dans la
# sortie de `make` ne le signale.
info "  CLEAN"
( cd "$SRC" && make clean > /dev/null 2>&1 ) || true

info "  MAKE  -j$(nproc)"
( cd "$SRC" && make -j"$(nproc)" > "$CHANTIER/make.log" 2>&1 ) || {
    rouge "make a echoue — voir $CHANTIER/make.log"
    tail -30 "$CHANTIER/make.log"
    exit 1
}

rm -rf "$PREFIXE"
( cd "$SRC" && make install > "$CHANTIER/install.log" 2>&1 ) || {
    rouge "make install a echoue — voir $CHANTIER/install.log"
    tail -20 "$CHANTIER/install.log"
    exit 1
}

VER=$(PKG_CONFIG_PATH="$PREFIXE/lib/pkgconfig" pkg-config --modversion icu-uc)
vert "  ok    ICU $VER dans $PREFIXE"
for a in "$PREFIXE"/lib/libicu{uc,i18n,data}.a; do
    printf '        %-16s %s\n' "$(basename "$a")" "$(du -h "$a" | cut -f1)"
done

# Verification qu'aucun `dlopen` n'a survecu : c'est la raison d'etre de
# `--disable-plugins`, et une option silencieusement ignoree par configure ne se
# verrait nulle part ailleurs.
if nm -u "$PREFIXE/lib/libicuuc.a" 2>/dev/null | grep -q '\bdlopen\b'; then
    rouge "  ATTENTION dlopen encore reference dans libicuuc.a"
    exit 1
fi
vert "  ok    aucune reference a dlopen"
