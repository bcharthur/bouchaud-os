#!/bin/sh
# Construit les binaires userland de Bouchaud OS.
#
# Trois chaines, de la plus simple a la plus complete :
#
#   ./build.sh freestanding   programmes de test sans libc (gcc + ld)
#   ./build.sh cpp23          temoin C++23 static-pie (prerequis portage Ladybird)
#   ./build.sh musl           binaires statiques musl (musl-gcc)
#   ./build.sh musl-dynamic   binaires dynamiques + ld-musl (ld.so)
#
# Contrainte commune : Bouchaud OS reserve un creneau d'adressage de 512 Gio a
# l'espace utilisateur, a partir de 0x400000000000. Un binaire lie a l'adresse
# Linux habituelle (0x400000) tomberait dans les tables de pages du noyau et
# serait refuse par le chargeur. Deux facons de s'y conformer :
#
#   - PIE / static-PIE : aucune adresse fixe, le noyau charge ou il veut
#     (recommande, c'est le defaut de la plupart des distributions) ;
#   - ELF non-PIE : passer -Wl,-Ttext-segment=0x400000000000, plus
#     -mcmodel=large car les adresses ne tiennent plus sur 32 bits.
#
# Les binaires produits sont a copier dans le RAMFS de l'OS puis a lancer avec
# la commande `exec`.

set -e
cd "$(dirname "$0")"
OUT=${OUT:-out}
BASE=0x400000400000
mkdir -p "$OUT"

CFLAGS_COMMON="-O2 -Wall -fno-stack-protector -fno-asynchronous-unwind-tables"
LDFLAGS_FIXED="-Wl,-Ttext-segment=$BASE -Wl,-z,noexecstack"

# Les en-tetes `linux/*` et `asm/*` (fb.h, input.h, kd.h, vt.h) decrivent l'ABI
# du noyau, pas la libc : musl ne les fournit pas. On les prend dans les
# en-tetes systeme, mais en `-idirafter` pour que ceux de musl restent
# prioritaires — sans quoi on melangerait deux libc.
UAPI_INCLUDES="-idirafter /usr/include -idirafter /usr/include/x86_64-linux-gnu"

freestanding() {
    # Sans libc : chaque appel systeme est ecrit a la main. Sert d'autotest de
    # l'ABI du noyau, independamment de toute bibliotheque.
    # -fno-builtin : sans libc, gcc ne doit pas remplacer une boucle par un
    # appel a strlen/memcpy qu'aucun objet ne fournirait.
    for src in ring3-selftest.c fb-demo.c; do
        name=$(basename "$src" .c)
        echo "  CC  $name"
        gcc -c $CFLAGS_COMMON -fno-builtin -mcmodel=large -fno-pie -mno-red-zone "$src" -o "$OUT/$name.o"
        ld -static -n -z noexecstack --no-warn-rwx-segments \
           -Ttext-segment=$BASE -e _start "$OUT/$name.o" -o "$OUT/$name"
        rm -f "$OUT/$name.o"
    done
}

musl_static() {
    command -v musl-gcc >/dev/null || {
        echo "musl-gcc introuvable (paquet musl-tools, ou construire musl depuis les sources)" >&2
        exit 1
    }
    # static-pie : pas d'adresse fixe, donc aucune contrainte d'edition de liens.
    # Necessite un musl >= 1.1.21 et binutils recent.
    for src in hello.c qpa-probe.c posix-probe.c verrous-probe.c exec-fd-probe.c wal-probe.c disque-probe.c nom-long-probe.c session-probe.c sendfile-probe.c audio-probe.c net-probe.c persist-probe.c shm-probe.c ipc-probe.c ordonnanceur-probe.c poll-bkl-probe.c exec-cible.c "$@"; do
        [ -f "$src" ] || continue
        name=$(basename "$src" .c)
        echo "  MUSL $name (static-pie)"
        musl-gcc $CFLAGS_COMMON $UAPI_INCLUDES -static-pie -pthread "$src" -o "$OUT/$name"
    done
}

musl_static_fixed() {
    # Repli si static-pie n'est pas disponible : adresse fixe dans le creneau.
    for src in hello.c "$@"; do
        [ -f "$src" ] || continue
        name=$(basename "$src" .c)
        echo "  MUSL $name (statique, base $BASE)"
        musl-gcc $CFLAGS_COMMON -mcmodel=large -static -no-pie $LDFLAGS_FIXED "$src" -o "$OUT/$name"
    done
}

# C++23 statique-PIE : la chaine que reclame Ladybird.
#
# C'est deja celle qui produit le navigateur (`build-navigateur.sh` : g++ +
# `-static-pie`), donc il n'y a pas de nouvelle chaine a installer — seulement a
# prouver qu'elle sait produire du C++23 qui *s'execute* en ring 3. Voir
# l'en-tete de `cpp23-probe.cpp` pour ce que la sonde verifie et pourquoi.
cpp23() {
    # clang++ et non g++ : AK emploie le parametre `this` explicite (22 fois),
    # que GCC 13 ne connait pas. GCC 14 le connait ; on prend donc le premier
    # compilateur disponible qui sache compiler la sonde.
    CXX_SONDE=""
    for c in "${CXX:-}" clang++ g++-14 g++; do
        [ -n "$c" ] || continue
        command -v "$c" >/dev/null || continue
        if echo 'struct S{int v;auto f(this S s){return s.v;}};int main(){return S{0}.f();}' \
           | "$c" -std=c++23 -x c++ - -o /dev/null 2>/dev/null; then
            CXX_SONDE=$c
            break
        fi
    done
    [ -n "$CXX_SONDE" ] && [ -n "$CXX_SONDE" ] || {
        echo "aucun compilateur C++23 avec parametre 'this' explicite (clang++ >= 17 ou g++ >= 14)" >&2
        exit 1
    }
    echo "  CXX  cpp23-probe ($CXX_SONDE, static-pie, -fno-exceptions)"
    "$CXX_SONDE" -std=c++23 -O2 -static-pie -fPIE -fno-exceptions -fno-rtti \
        -Wall -Wextra cpp23-probe.cpp -o "$OUT/cpp23-probe"
}

musl_dynamic() {
    command -v musl-gcc >/dev/null || { echo "musl-gcc introuvable" >&2; exit 1; }
    for src in hello.c "$@"; do
        [ -f "$src" ] || continue
        name=$(basename "$src" .c)
        echo "  MUSL $name (dynamique)"
        musl-gcc $CFLAGS_COMMON -fPIE -pie "$src" -o "$OUT/$name"
    done
    # L'interpreteur doit exister dans le RAMFS au chemin exact indique par
    # PT_INTERP (voir `elfinfo <binaire>` dans le shell de l'OS).
    for candidate in /lib/ld-musl-x86_64.so.1 /usr/lib/musl/lib/libc.so; do
        if [ -f "$candidate" ]; then
            echo "  CP  ld-musl-x86_64.so.1"
            cp "$candidate" "$OUT/ld-musl-x86_64.so.1"
            break
        fi
    done
}

case "${1:-freestanding}" in
    freestanding) freestanding ;;
    cpp23) cpp23 ;;
    musl) musl_static || musl_static_fixed ;;
    musl-fixed) musl_static_fixed ;;
    musl-dynamic) musl_dynamic ;;
    *) echo "usage: $0 [freestanding|cpp23|musl|musl-fixed|musl-dynamic]" >&2; exit 2 ;;
esac

echo "binaires dans $OUT/ — a copier dans le RAMFS puis lancer avec 'exec'"
