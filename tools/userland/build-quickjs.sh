#!/bin/bash
# Construit QuickJS en bibliotheque statique, pour le moteur JavaScript du
# navigateur.
#
#   ./build-quickjs.sh
#
# Produit `out-quickjs/lib/libquickjs.a` et les en-tetes, que
# `build-navigateur.sh` lie dans `bo-navigateur`.
#
# ## Pourquoi QuickJS
#
# Ecrire un interprete JavaScript complet — analyse, machine virtuelle,
# ramasse-miettes, expressions rationnelles, Unicode, `Promise` — represente
# plus de travail que tout le reste du navigateur reuni. QuickJS fait tout cela
# en C portable, sans dependance, et se compile en statique : c'est exactement
# la forme dont l'OS a besoin, puisqu'il ne sait charger ni bibliotheque
# partagee ni greffon.
#
# ## Le cœur seul
#
# On ne compile que le moteur de langage (`quickjs.c`, `libregexp.c`,
# `libunicode.c`, `cutils.c`, `libbf.c`). `quickjs-libc.c` — qui apporterait
# `os.*`, `std.*`, les fichiers et les processus — est volontairement laisse de
# cote : une page web n'a rien a faire avec le systeme de fichiers, et tout ce
# que le JavaScript peut toucher passe par le pont ecrit dans `hote.cpp`.
#
# ## Ou vient la source
#
# Depuis la distribution source du paquet PyPI `quickjs`, qui embarque l'archive
# amont telle quelle (`upstream-quickjs/`). C'est un miroir accessible depuis
# cette machine, contrairement a bellard.org.

set -e
cd "$(dirname "$0")"
ROOT=$PWD

OUT=${OUT:-out-quickjs}
WORK=${WORK:-build-quickjs}
PYPI_VER=${PYPI_VER:-1.19.4}
case "$WORK" in /*) ;; *) WORK=$ROOT/$WORK ;; esac
case "$OUT" in /*) ;; *) OUT=$ROOT/$OUT ;; esac

mkdir -p "$WORK"
cd "$WORK"

# --- Source -----------------------------------------------------------------
if [ ! -d "quickjs-$PYPI_VER/upstream-quickjs" ]; then
    echo "== recuperation de QuickJS =="
    if [ ! -f quickjs.tar.gz ]; then
        URL=$(curl -sf --max-time 60 "https://pypi.org/pypi/quickjs/$PYPI_VER/json" | python3 -c "
import json, sys
d = json.load(sys.stdin)
print([f['url'] for f in d['urls'] if f['packagetype'] == 'sdist'][0])")
        curl -sfL --max-time 300 -o quickjs.tar.gz "$URL"
    fi
    tar xzf quickjs.tar.gz
fi

SRC=$WORK/quickjs-$PYPI_VER/upstream-quickjs
VERSION=$(cat "$SRC/VERSION")
echo "== QuickJS $VERSION =="

# --- Compilation ------------------------------------------------------------
# `-fPIE` : le binaire final est un static-pie, tous ses objets doivent l'etre.
# `CONFIG_BIGNUM` : demande par `libbf.c`, et c'est ce qui donne `BigInt`.
# `_GNU_SOURCE` : quickjs.c se sert de `sincos` et de `__builtin` glibc.
CFLAGS="-O2 -fPIE -fwrapv -flto=auto -D_GNU_SOURCE -DCONFIG_BIGNUM"
CFLAGS="$CFLAGS -DCONFIG_VERSION=\"$VERSION\""
# Le moteur est genere : ses avertissements ne nous apprennent rien.
CFLAGS="$CFLAGS -Wno-array-bounds -Wno-format-truncation -Wno-unused-but-set-variable"

mkdir -p obj
for unite in quickjs libregexp libunicode cutils libbf; do
    printf '  %s.c\n' "$unite"
    # shellcheck disable=SC2086
    gcc $CFLAGS -I"$SRC" -c "$SRC/$unite.c" -o "obj/$unite.o"
done

rm -f "obj/libquickjs.a"
ar rcs "obj/libquickjs.a" obj/quickjs.o obj/libregexp.o obj/libunicode.o \
       obj/cutils.o obj/libbf.o

# --- Empaquetage ------------------------------------------------------------
rm -rf "$OUT"
mkdir -p "$OUT/lib" "$OUT/include"
cp "obj/libquickjs.a" "$OUT/lib/"
cp "$SRC/quickjs.h" "$SRC/quickjs-atom.h" "$SRC/quickjs-opcode.h" \
   "$SRC/libregexp.h" "$SRC/libregexp-opcode.h" "$SRC/libunicode.h" \
   "$SRC/cutils.h" "$SRC/libbf.h" "$SRC/list.h" "$OUT/include/"
echo "$VERSION" > "$OUT/VERSION"

echo ""
echo "pret dans $OUT/ :"
ls -la "$OUT/lib/libquickjs.a" | sed 's/^/  /'
