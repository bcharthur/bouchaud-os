#!/bin/bash
# Construit LibJS et ses bibliotheques satellites, et joue un temoin.
#
#   ./tools/ladybird/build-libjs.sh          hote
#   ./tools/ladybird/build-libjs.sh --cible  Bouchaud (static-pie)
#
# ## Ce que ce jalon prouve
#
# Que la chaine complete tient : C++23, AK, LibCore, LibGC, LibUnicode avec ses
# donnees ICU, LibCrypto pour `BigInt`, cinq caisses Rust, et 261 sources de
# LibJS. Le temoin evalue du JavaScript et imprime le resultat.
#
# ## Le generateur de code-octet
#
# LibJS ne compile pas tel quel : `Bytecode/Op.h` et `Bytecode/OpCodes.h`
# n'existent pas dans l'arbre. Ils sont produits par `flapc`, un compilateur
# ecrit en Rust, a partir de `Interpreter/interpreter.flap` — 3 568 lignes qui
# decrivent l'interpreteur dans un langage dedie.
#
# C'est la deuxieme chaine Rust du projet, et elle n'est pas dans l'espace de
# travail : `Cargo.toml` a la racine exclut explicitement `Libraries/LibJS/Flap`.
# Il faut donc l'appeler par son manifeste.
#
# ## Les satellites
#
# `LibJS` lie en prive LibTextCodec, LibRegex, LibSyntax, LibFileSystem et
# LibURL — seize sources en tout, mesurees et non supposees. Elles sont
# construites ici plutot que dans cinq scripts d'une source chacun.

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
GC="$RACINE/third_party/build-libgc-$CIBLE"
UNI="$RACINE/third_party/build-libunicode-$CIBLE"
CRYPTO="$RACINE/third_party/build-libcrypto-$CIBLE"
RUST="$RACINE/third_party/build-rust-$CIBLE"
ICU="$RACINE/third_party/icu-$CIBLE"
SORTIE="$RACINE/third_party/build-libjs-$CIBLE"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

[ -d "$GEN/LibJS" ] || { rouge "en-tetes absents — lancer build-entetes.sh ${1:-}"; exit 1; }
for brique in "$GC/libGC.a" "$UNI/libUnicode.a" "$CRYPTO/libCryptoMin.a" "$CORE/libCore.a"; do
    [ -f "$brique" ] || { rouge "$(basename "$brique") absent — voir docs/ladybird/LIBJS_PORT.md"; exit 1; }
done
TRIPLET=x86_64-unknown-linux-gnu
RUSTLIB="$RUST/$TRIPLET/release"
[ -f "$RUSTLIB/liblibjs_rust.a" ] \
    || { rouge "caisses Rust absentes — lancer build-rust.sh ${1:-}"; exit 1; }

info "== LibJS ($CIBLE) =="

mkdir -p "$SORTIE/obj" "$SORTIE/gen/LibJS/Bytecode"

# --- Le compilateur de code-octet -------------------------------------------
#
# `Libraries/LibJS/Flap` est **exclu** de l'espace de travail par le
# `Cargo.toml` de la racine : on l'appelle donc par `--manifest-path`, et avec
# son propre `CARGO_TARGET_DIR` pour ne pas melanger ses artefacts a ceux des
# caisses de bibliotheque.
#
# ## Pourquoi le `cd` dans l'arbre Ladybird n'est pas decoratif
#
# `--manifest-path` suffirait a cargo, mais pas a rustup, et la difference
# coute une erreur incomprehensible :
#
#     error[E0152]: duplicate lang item in crate `core`
#                   (which `smallvec` depends on): `sized`
#
# Rien dans ce message ne parle de chaine d'outils. Voici pourtant ce qui se
# passe. Depuis la racine de Bouchaud, rustup selectionne la **nuitee** du
# noyau, et `.cargo/config.toml` y demande `build-std = ["core", ...]` : cargo
# recompile alors `core` depuis les sources. Mais `smallvec`, dependance de
# `flapc`, est lie au `core` **precompile** de la chaine. Deux `core` dans un
# meme graphe, et chacun declare les memes elements de langage.
#
# Depuis l'arbre Ladybird, `rust-toolchain.toml` selectionne la version stable
# 1.96.1, qui ignore la section `[unstable]` : `build-std` ne s'applique pas, il
# n'y a qu'un seul `core`, et tout se compile. C'est la meme raison qui fait
# fonctionner `build-rust.sh`, ou le `cd` etait deja present.
#
# La lecon generale : le repertoire courant selectionne la chaine d'outils
# Rust, et Bouchaud en heberge deux — celle du noyau et celle de Ladybird. Toute
# invocation de cargo pour Ladybird doit partir de son arbre.
FLAPC="$SORTIE/flapc/$TRIPLET/release/generate-libjs-bytecode"
if [ ! -x "$FLAPC" ]; then
    info "  CARGO flapc (generateur de code-octet)"
    ( cd "$LB"
      CARGO_BUILD_TARGET="$TRIPLET" \
      CARGO_TARGET_DIR="$SORTIE/flapc" \
          cargo build --release --quiet \
              --manifest-path "$LB/Libraries/LibJS/Flap/Cargo.toml" \
              --bin generate-libjs-bytecode )
fi

if [ ! -f "$SORTIE/gen/LibJS/Bytecode/Op.h" ]; then
    info "  GEN   Bytecode/Op.h, Bytecode/OpCodes.h"
    "$FLAPC" \
        --input "$LB/Libraries/LibJS/Interpreter/interpreter.flap" \
        --op-header "$SORTIE/gen/LibJS/Bytecode/Op.h" \
        --opcodes-header "$SORTIE/gen/LibJS/Bytecode/OpCodes.h"
fi
vert "  ok    Op.h ($(wc -l < "$SORTIE/gen/LibJS/Bytecode/Op.h") lignes)"

# --- L'interpreteur lui-meme, en assembleur ---------------------------------
#
# C'est la partie la plus inattendue de LibJS, et l'editeur de liens est le
# premier a en parler — par un `undefined reference to
# js_interpreter_handler_ranges` que rien dans les sources C++ n'explique.
#
# L'interpreteur de code-octet de LibJS n'est pas ecrit en C++ : il est **genere
# en assembleur** a partir du meme `interpreter.flap`, par le binaire `flapc`
# (distinct de `generate-libjs-bytecode`, qui ne produit que les en-tetes). Le
# fichier `.S` contient un gestionnaire par instruction, et la table
# `js_interpreter_handler_ranges` qui les delimite.
#
# Cela suppose que l'assembleur connaisse la disposition memoire des structures
# C++ qu'il manipule. D'ou la troisieme piece : `Interpreter/GenerateLayout.cpp`,
# un petit programme C++ compile avec `-Dprivate=public -Dprotected=public` — de
# sorte qu'`offsetof` puisse atteindre les champs prives — et dont la sortie est
# la liste des decalages, relue par `flapc`.
#
#     interpreter.flap ─┐
#                       ├─> flapc ─> interpreter_x86_64.S ─> .o
#     GenerateLayout.cpp┘   (via layout.conf)
#
# Trois programmes generes doivent donc tourner avant que LibJS existe.
GENLAYOUT="$SORTIE/generate_interpreter_layout"
if [ ! -x "$GENLAYOUT" ]; then
    info "  CXX   generate_interpreter_layout"
    ${CXX:-clang++} -std=c++23 -O2 -fno-exceptions $FLAGS_CIBLE \
        -Dprivate=public -Dprotected=public \
        -I"$LB" -I"$LB/Libraries" -I"$AK/gen" -I"$GEN" -I"$SORTIE/gen" \
        -I"$DEPS/include" -I"$ICU/include" \
        -Wno-unused-parameter -Wno-unknown-pragmas -Wno-invalid-constexpr \
        -Wno-unqualified-std-cast-call -Wno-user-defined-literals \
        -Wno-unknown-warning-option -Wno-keyword-macro \
        "$LB/Libraries/LibJS/Interpreter/GenerateLayout.cpp" -o "$GENLAYOUT" \
        "$AK/libAK.a" -L"$DEPS/lib" -lfmt -lsimdutf -lmimalloc -lpthread
fi

if [ ! -f "$SORTIE/gen/layout.conf" ]; then
    info "  GEN   layout.conf (decalages des structures)"
    "$GENLAYOUT" > "$SORTIE/gen/layout.conf"
fi

FLAPC_BIN="$SORTIE/flapc/$TRIPLET/release/flapc"
if [ ! -x "$FLAPC_BIN" ]; then
    info "  CARGO flapc (interpreteur)"
    ( cd "$LB"
      CARGO_BUILD_TARGET="$TRIPLET" \
      CARGO_TARGET_DIR="$SORTIE/flapc" \
          cargo build --release --quiet \
              --manifest-path "$LB/Libraries/LibJS/Flap/Cargo.toml" \
              --bin flapc )
fi

INTERP_S="$SORTIE/gen/interpreter_x86_64.S"
if [ ! -f "$INTERP_S" ]; then
    info "  GEN   interpreter_x86_64.S"
    "$FLAPC_BIN" --arch x86_64 --object-format elf \
        --constants "$SORTIE/gen/layout.conf" \
        --input "$LB/Libraries/LibJS/Interpreter/interpreter.flap" \
        --output "$INTERP_S"
fi
vert "  ok    interpreter_x86_64.S ($(wc -l < "$INTERP_S") lignes)"

INTERP_O="$SORTIE/obj/interpreter_x86_64.o"
if [ ! -f "$INTERP_O" ] || [ "$INTERP_S" -nt "$INTERP_O" ]; then
    info "  AS    interpreter_x86_64.S"
    # Les deux options d'alignement viennent d'upstream : elles maintiennent les
    # branches conditionnelles a l'interieur de fenetres de 32 octets, pour que
    # l'attenuation microcode d'Intel n'empeche pas leurs blocs d'utiliser le
    # tampon de decodage. C'est une optimisation, pas une correction — mais la
    # retirer changerait silencieusement les performances de l'interpreteur.
    ${CXX:-clang++} $FLAGS_CIBLE \
        -malign-branch-boundary=32 -malign-branch=fused,jcc \
        -c "$INTERP_S" -o "$INTERP_O"
fi

CXX=${CXX:-clang++}
# `-DAK_SYSTEM_CACHE_ALIGNMENT_SIZE=64` : voir build-ak.sh.
CXXFLAGS="-std=c++23 -O2 -fno-exceptions -fPIC -DAK_SYSTEM_CACHE_ALIGNMENT_SIZE=64 $FLAGS_CIBLE \
    -I$LB -I$LB/Libraries \
    -I$AK/gen -I$GEN -I$SORTIE/gen \
    -I$DEPS/include -I$ICU/include \
    -Wno-unused-parameter -Wno-unknown-pragmas -Wno-invalid-constexpr \
    -Wno-unqualified-std-cast-call -Wno-user-defined-literals \
    -Wno-unknown-warning-option"

# --- Compilation ------------------------------------------------------------
#
# La liste des sources vient des `CMakeLists.txt` d'upstream, jamais d'une copie
# qui derive. Quand upstream ajoute un fichier, on le prend sans rien modifier
# ici.
sources_de() {
    sed -n '/^set(SOURCES/,/^)/p' "$LB/Libraries/$1/CMakeLists.txt" \
        | grep -oE '[A-Za-z0-9_/]+\.cpp'
}

# Les marques d'echec de la construction precedente : sans ce nettoyage, un
# fichier repare continuerait d'etre signale en echec.
rm -f "$SORTIE"/obj/*.echec

ECHECS=0
OBJETS=""
PARALLELE=$(nproc)

compile_biblio() {
    local biblio=$1
    local liste
    liste=$(sources_de "$biblio")
    local nb
    nb=$(echo "$liste" | wc -w)
    info "  $biblio : $nb fichier(s)"

    # L'en-tete FFI de *cette* bibliotheque, et d'aucune autre : cinq fichiers
    # portent le meme nom, et un `-I` commun donnerait a chacune l'interface
    # d'une autre. Voir build-entetes.sh.
    local ffi=""
    [ -d "$GEN/ffi/$biblio" ] && ffi="-I$GEN/ffi/$biblio"

    local en_vol=0
    for src in $liste; do
        local obj="$SORTIE/obj/$biblio-$(echo "${src%.cpp}" | tr / -).o"
        OBJETS="$OBJETS $obj"
        if [ -f "$obj" ] && [ "$obj" -nt "$LB/Libraries/$biblio/$src" ]; then
            continue
        fi
        $CXX $CXXFLAGS $ffi -c "$LB/Libraries/$biblio/$src" -o "$obj" \
            2> "${obj%.o}.log" || touch "${obj%.o}.echec" &
        en_vol=$((en_vol + 1))
        if [ "$en_vol" -ge "$PARALLELE" ]; then
            wait
            en_vol=0
        fi
    done
    wait
}

# Les satellites d'abord : s'ils echouent, inutile d'attendre 261 fichiers pour
# l'apprendre.
for biblio in LibTextCodec LibSyntax LibFileSystem LibRegex LibURL LibJS; do
    compile_biblio "$biblio"
done

for marque in "$SORTIE"/obj/*.echec; do
    [ -e "$marque" ] || break
    base=${marque%.echec}
    rouge "  ECHEC $(basename "$base")"
    grep -m2 "error:" "$base.log" 2>/dev/null | sed 's/^/          /'
    ECHECS=$((ECHECS + 1))
done

if [ "$ECHECS" -gt 0 ]; then
    rouge "$ECHECS fichier(s) n'ont pas compile"
    rouge "  (les marques .echec restent dans $SORTIE/obj pour inspection)"
    exit 1
fi

ar rcs "$SORTIE/libJS.a" $OBJETS "$INTERP_O"
vert "  ok    libJS.a ($(du -h "$SORTIE/libJS.a" | cut -f1))"

# --- Le temoin --------------------------------------------------------------
#
# L'ordre des archives compte : l'editeur de liens ne revient pas en arriere.
# LibJS avant LibGC avant LibUnicode avant LibCore avant AK, puis le tiers-parti,
# et `libicudata` en dernier — elle ne reference rien, elle est referencee.
#
# ## `--allow-multiple-definition`, et pourquoi ce n'est pas un pansement
#
# Les cinq caisses Rust sont declarees `crate-type = ["staticlib"]`. Une
# staticlib Rust est concue pour etre **autonome** : elle embarque sa propre
# copie de la bibliotheque standard, y compris les relais d'allocation
# `__rust_alloc`, `__rust_dealloc`, `__rust_realloc`, `__rust_alloc_zeroed`.
# Cinq archives, donc cinq definitions des memes symboles.
#
# Upstream ne rencontre jamais ce conflit : `ladybird_lib()` produit des
# bibliotheques **partagees**, ou chaque `.so` garde sa copie sans en parler a
# personne. Bouchaud, lui, produit un binaire `-static-pie` unique — tout
# atterrit dans la meme unite de liaison.
#
# Le drapeau fait ici exactement ce qu'on veut, et pas seulement « taire
# l'erreur » : l'editeur de liens retient la **premiere** definition et y fait
# pointer toutes les references. Il n'y a donc pas cinq allocateurs qui
# coexistent, mais un seul pour tout le code Rust — ce qui est precisement la
# propriete qu'il faut, puisqu'une zone allouee par une caisse et liberee par
# une autre traverserait sinon deux tas.
#
# Reste a choisir laquelle gagne, et ce n'est pas indifferent : les caisses
# construites avec la fonctionnalite `allocator` renvoient vers
# `ladybird_rust_alloc`, donc vers le tas d'AK, tandis que les autres emploient
# l'allocateur systeme. `libunicode_rust` est donc placee **en tete** parce
# qu'elle porte cette fonctionnalite. Le choix est verifie apres l'edition de
# liens plutot que suppose — voir la verification `nm` qui suit.
info "  CXX   temoin LibJS"
$CXX $CXXFLAGS "$RACINE/tools/ladybird/libjs-probe.cpp" -o "$SORTIE/libjs-probe" \
    "$SORTIE/libJS.a" \
    "$GC/libGC.a" "$UNI/libUnicode.a" "$CRYPTO/libCryptoMin.a" \
    "$CORE/libCore.a" "$AK/libAK.a" \
    "$RUSTLIB/liblibunicode_rust.a" "$RUSTLIB/liblibregex_rust.a" \
    "$RUSTLIB/libliburl_rust.a" \
    "$RUSTLIB/liblibjs_rust.a" "$RUSTLIB/liblibtextcodec_rust.a" \
    -Wl,--allow-multiple-definition \
    -L"$DEPS/lib" -lsimdjson -ltommath -lpsl -lfmt -lsimdutf -lmimalloc \
    -L"$ICU/lib" -licui18n -licuuc -licudata \
    -lpthread -lm -ldl

vert "temoin construit : $SORTIE/libjs-probe ($(du -h "$SORTIE/libjs-probe" | cut -f1))"

# --- Quel allocateur Rust a gagne ? -----------------------------------------
#
# `--allow-multiple-definition` retient une definition et ecarte les autres en
# silence. Verifier laquelle n'est donc pas un luxe : si celle de l'allocateur
# systeme l'emportait, le code Rust allouerait hors du tas d'AK sans qu'aucun
# message ne le signale, et les deux tas coexisteraient jusqu'au premier
# transfert de propriete entre Rust et C++.
#
# `__rust_alloc` doit aboutir a `ladybird_rust_alloc` — la fonction de
# `AK/kmalloc.cpp`.
#
# Le desassemblage seul ne suffit pas a le dire : le relais compile est un saut
# **indirect** a travers la table des adresses globales
# (`jmp *0x…(%rip)`), et le nom de la cible n'y figure donc pas. Chercher
# « ladybird_rust_alloc » dans la sortie d'`objdump` repondrait toujours non,
# y compris quand tout va bien — une verification qui echoue toujours ne vaut
# pas mieux qu'une verification absente.
#
# On suit donc la chaine en entier : l'adresse de l'emplacement vise, puis sa
# relocation, puis le symbole qui se trouve a l'adresse ainsi obtenue.
verifie_allocateur_rust() {
    command -v objdump > /dev/null && command -v readelf > /dev/null || return 0

    local emplacement cible attendu
    emplacement=$(objdump -d "$SORTIE/libjs-probe" 2>/dev/null \
        | grep -A2 '<[^>]*__rust_alloc>:' \
        | grep -om1 '# [0-9a-f]* ' | tr -d '# ')
    [ -n "$emplacement" ] || { info "  note  relais d'allocation Rust introuvable"; return 0; }

    cible=$(readelf -r "$SORTIE/libjs-probe" 2>/dev/null \
        | awk -v e="$emplacement" 'tolower($1) ~ ("^0*" e "$") { print $NF }' | tail -1)
    attendu=$(nm "$SORTIE/libjs-probe" 2>/dev/null \
        | awk '$3 == "ladybird_rust_alloc" { print $1 }')

    if [ -n "$cible" ] && [ -n "$attendu" ] \
       && [ "$((16#$cible))" = "$((16#$attendu))" ]; then
        vert "  ok    l'allocateur Rust retenu passe par le tas d'AK"
    else
        rouge "  ATTENTION l'allocateur Rust retenu n'aboutit pas a ladybird_rust_alloc"
        rouge "           vise 0x${cible:-?}, attendu 0x${attendu:-?}"
        rouge "           le code Rust allouerait hors du tas d'AK — voir l'ordre des archives"
    fi
}
verifie_allocateur_rust
if [ "$CIBLE" = "hote" ]; then
    echo
    "$SORTIE/libjs-probe"
fi
