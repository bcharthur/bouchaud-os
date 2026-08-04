#!/bin/bash
# Construit un CPython 3.12 statique-PIE pour Bouchaud OS.
#
# Resultat : `out-python/usr/bin/python3` (un seul fichier, ~10 Mio) et
# `out-python/usr/lib/python312.zip` (la bibliotheque standard, ~2,5 Mio).
# A deposer sur le disque de donnees avec mkdisk.sh — le noyau n'a pas besoin
# d'etre recompile.
#
#   ./build-python.sh              telecharge, construit, empaquette
#   PYVER=3.12.3 ./build-python.sh choisit une autre version
#
# Pourquoi statique : l'OS sait charger un `ld.so`, mais un interprete statique
# n'a besoin de rien d'autre que lui-meme — pas de bibliotheque a placer au bon
# chemin, pas de version a faire correspondre. C'est aussi ce qui impose que
# toutes les extensions C soient liees dans le binaire : un executable statique
# ne peut pas charger de `.so`.
#
# Pourquoi PIE : Bouchaud OS reserve un creneau d'adressage a 0x400000000000 et
# refuse un binaire lie a l'adresse Linux habituelle (0x400000). Un static-PIE
# n'a pas d'adresse fixe : le noyau le charge ou il veut.

set -e
cd "$(dirname "$0")"
ROOT=$PWD

PYVER=${PYVER:-3.12.3}
PYSHORT=$(echo "$PYVER" | cut -d. -f1,2)     # 3.12
PYTAG=$(echo "$PYSHORT" | tr -d .)           # 312
OUT=${OUT:-out-python}
WORK=${WORK:-build-python}
# Un chemin relatif est compris depuis ce repertoire ; un chemin absolu est
# garde tel quel, ce qui permet de construire ailleurs que dans les sources.
case "$WORK" in /*) ;; *) WORK=$ROOT/$WORK ;; esac
case "$OUT" in /*) ;; *) OUT=$ROOT/$OUT ;; esac
SYSROOT=$WORK/sysroot

# Ubuntu publie les sources amont telles quelles ; c'est le miroir le plus
# accessible depuis une machine de construction Debian/Ubuntu.
PYURL=${PYURL:-http://archive.ubuntu.com/ubuntu/pool/main/p/python$PYSHORT/python${PYSHORT}_${PYVER}.orig.tar.xz}
ZLIBURL=${ZLIBURL:-http://archive.ubuntu.com/ubuntu/pool/main/z/zlib/zlib_1.3.dfsg+really1.3.1.orig.tar.gz}

# `LIBC=musl` (defaut) : interprete autonome, sans rien partager avec quoi que
# ce soit d'autre. `LIBC=glibc` : necessaire des que l'interprete doit etre lie
# a du C++ — Qt notamment, dont la libstdc++ est celle du systeme et donc liee a
# la glibc. Les deux libc ne se melangent pas dans un meme binaire.
CIBLE_LIBC=${LIBC:-musl}
# `LIBC` est aussi une variable du Makefile de CPython (la bibliotheque a
# ajouter a l'edition de liens). La laisser dans l'environnement la ferait
# passer telle quelle au configure, qui reclamerait alors un « -lglibc ».
unset LIBC

mkdir -p "$WORK"
cd "$WORK"

if [ "$CIBLE_LIBC" = musl ]; then
    command -v musl-gcc >/dev/null || {
        echo "musl-gcc introuvable (paquet musl-tools)" >&2
        exit 1
    }
    # --- Habillage de musl-gcc ----------------------------------------------
    # musl-gcc est un habillage du gcc systeme : interroge sur son « multiarch »,
    # il repond « x86_64-linux-gnu » alors que la cible est musl. Le configure de
    # CPython compare les deux et s'arrete sur la contradiction. On le fait taire.
    mkdir -p bin
    cat > bin/mgcc <<'EOF'
#!/bin/sh
for a in "$@"; do
    case "$a" in --print-multiarch|-print-multiarch) exit 0 ;; esac
done
exec musl-gcc "$@"
EOF
    chmod +x bin/mgcc
    CC=$PWD/bin/mgcc
else
    CC=gcc
fi

# --- zlib -------------------------------------------------------------------
# Necessaire a `zipimport` pour lire une bibliotheque standard compressee. En
# musl il faut la reconstruire : celle du systeme est liee a la glibc.
if [ "$CIBLE_LIBC" = musl ] && [ ! -f "$SYSROOT/lib/libz.a" ]; then
    echo "== zlib =="
    [ -f zlib.tar.gz ] || curl -sL -o zlib.tar.gz "$ZLIBURL"
    rm -rf zlib-src && mkdir zlib-src
    tar xf zlib.tar.gz -C zlib-src --strip-components=1
    (cd zlib-src && CC="$CC" CFLAGS="-O2 -fPIE" ./configure --prefix="$SYSROOT" --static >/dev/null \
        && make -j"$(nproc)" >/dev/null && make install >/dev/null)
elif [ "$CIBLE_LIBC" = glibc ]; then
    # En glibc, celle du systeme convient : on la prend telle quelle.
    mkdir -p "$SYSROOT/include" "$SYSROOT/lib"
fi

# --- CPython ----------------------------------------------------------------
echo "== CPython $PYVER =="
[ -f python.tar.xz ] || curl -sL -o python.tar.xz "$PYURL"
if [ ! -d "Python-$PYVER" ]; then
    tar xf python.tar.xz
fi
cd "$WORK/Python-$PYVER"

# musl-gcc voit quand meme /usr/include : configure y trouve lzma.h, sqlite3.h,
# ffi.h... dont les bibliotheques, elles, sont liees a la glibc. Les melanger
# donnerait un binaire qui ne se lie pas. Aucun de ces modules n'est necessaire
# pour faire tourner un interprete.
cat > Modules/Setup.local <<'SETUP'
*static*

*disabled*
_lzma
_bz2
_sqlite3
_curses
_curses_panel
_tkinter
_uuid
_ctypes
_ctypes_test
readline
nis
ossaudiodev
_gdbm
_dbm
_crypt
_testcapi
_testinternalcapi
_testbuffer
_testimportmultiple
_testmultiphase
_testsinglephase
_xxtestfuzz
xxlimited
xxlimited_35
SETUP

# TLS : en glibc, l'OpenSSL statique du systeme convient et donne un `ssl`
# complet — c'est ce qui permet au navigateur de parler HTTPS. En musl il
# faudrait reconstruire OpenSSL contre musl : hors sujet pour un interprete
# autonome, on s'en passe.
if [ "$CIBLE_LIBC" = musl ]; then
    printf '_ssl\n_hashlib\n' >> Modules/Setup.local
fi

# En musl on produit un interprete autonome, donc lie en static-pie. En glibc on
# ne cherche que `libpython3.12.a`, destinee a etre embarquee dans le navigateur
# (c'est lui qui sera lie en static-pie) : l'executable intermediaire, lui, doit
# simplement tourner sur la machine de construction pendant le `make`. Le lier en
# static-pie le ferait justement segfauter des la premiere etape qui l'execute.
if [ "$CIBLE_LIBC" = musl ]; then
    LIAISON="-static-pie -L$SYSROOT/lib"
else
    LIAISON=""
fi

if [ ! -f Makefile ]; then
    CC="$CC" \
    CFLAGS="-O2 -fPIE -fno-stack-protector -I$SYSROOT/include" \
    CPPFLAGS="-I$SYSROOT/include" \
    LDFLAGS="$LIAISON" \
    ./configure \
        --prefix=/usr \
        --disable-shared \
        --without-ensurepip \
        --with-ensurepip=no \
        --disable-test-modules \
        --with-static-libpython \
        --disable-ipv6 \
        --with-openssl=/usr \
        ac_cv_func_dlopen=no \
        ac_cv_lib_dl_dlopen=no \
        MODULE_BUILDTYPE=static \
        > configure.log 2>&1 || { tail -20 configure.log; exit 1; }
fi

make -j"$(nproc)" > make.log 2>&1 || { grep -m5 "error:" make.log; exit 1; }
rm -rf staged && make install DESTDIR="$PWD/staged" > install.log 2>&1

# --- Empaquetage ------------------------------------------------------------
cd "$ROOT"
STAGED=$WORK/Python-$PYVER/staged/usr
DEST=$OUT

rm -rf "$DEST"
mkdir -p "$DEST/usr/bin" "$DEST/usr/lib"
if [ "$CIBLE_LIBC" = musl ]; then
    strip -s "$STAGED/bin/python$PYSHORT" -o "$DEST/usr/bin/python3"
else
    # Variante d'embarquement : ni interprete autonome, ni depouillement — la
    # bibliotheque et les en-tetes servent a lier Python dans un autre binaire.
    cp "$WORK/Python-$PYVER/libpython$PYSHORT.a" "$DEST/usr/lib/"
    cp -r "$STAGED/include/python$PYSHORT" "$DEST/usr/include" 2>/dev/null \
        || cp -r "$WORK/Python-$PYVER/Include" "$DEST/usr/include"
    cp "$WORK/Python-$PYVER/pyconfig.h" "$DEST/usr/include/" 2>/dev/null || true
    # `libpython.a` ne contient pas tout : CPython lie separement les objets des
    # bibliotheques tierces qu'il embarque — expat pour `pyexpat`, HACL* pour
    # les fonctions de hachage, libmpdec pour `_decimal`. Sans elles, l'edition
    # de liens de l'hote echoue sur des symboles absents. On les rassemble en
    # une archive d'appoint.
    rm -f "$DEST/usr/lib/libpythonaux.a"
    ar rcs "$DEST/usr/lib/libpythonaux.a" \
        "$WORK/Python-$PYVER"/Modules/expat/*.o \
        "$WORK/Python-$PYVER"/Modules/_hacl/*.o \
        "$WORK/Python-$PYVER"/Modules/_decimal/libmpdec/*.o
fi

# La bibliotheque standard en une archive zip plutot qu'en 2000 fichiers :
# c'est ce que `zipimport` sait lire, et c'est autant de nœuds que le RAMFS
# n'a pas a creer au demarrage. CPython la cherche tout seul a
# `<prefixe>/lib/python312.zip`.
python3 - "$STAGED/lib/python$PYSHORT" "$DEST/usr/lib/python$PYTAG.zip" <<'EOF'
import os, sys, zipfile
source, cible = sys.argv[1], sys.argv[2]
# Les repertoires de tests et l'interface graphique Tk ne servent a rien ici et
# pesent plus que tout le reste.
exclus = {"test", "tests", "idlelib", "tkinter", "turtledemo", "__pycache__",
          "pydoc_data", "ensurepip", "lib2to3"}
z = zipfile.ZipFile(cible, "w", zipfile.ZIP_DEFLATED)
n = 0
for racine, dossiers, fichiers in os.walk(source):
    dossiers[:] = [d for d in dossiers if d not in exclus]
    for f in fichiers:
        if f.endswith(".py"):
            chemin = os.path.join(racine, f)
            z.write(chemin, os.path.relpath(chemin, source))
            n += 1
z.close()
print("  %d modules -> %s (%d Kio)" % (n, os.path.basename(cible),
                                       os.path.getsize(cible) // 1024))
EOF

echo ""
echo "pret dans $OUT/ :"
if [ "$CIBLE_LIBC" = musl ]; then
    ls -la "$DEST/usr/bin/python3" "$DEST/usr/lib/python$PYTAG.zip"
    echo ""
    echo "fabriquer le disque :   ./mkdisk.sh $OUT"
    echo "puis, sous l'OS     :   exec /usr/bin/python3 /mon-script.py"
else
    ls -la "$DEST/usr/lib/libpython$PYSHORT.a" "$DEST/usr/lib/python$PYTAG.zip"
    echo ""
    echo "a lier dans un binaire hote (voir build-navigateur.sh)"
fi
