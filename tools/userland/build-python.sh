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

command -v musl-gcc >/dev/null || {
    echo "musl-gcc introuvable (paquet musl-tools)" >&2
    exit 1
}

mkdir -p "$WORK"
cd "$WORK"

# --- Habillage de musl-gcc --------------------------------------------------
# musl-gcc est un habillage du gcc systeme : interroge sur son « multiarch », il
# repond « x86_64-linux-gnu » alors que la cible est musl. Le configure de
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

# --- zlib -------------------------------------------------------------------
# Necessaire a `zipimport` pour lire une bibliotheque standard compressee. Celle
# du systeme est liee a la glibc : il faut la reconstruire contre musl.
if [ ! -f "$SYSROOT/lib/libz.a" ]; then
    echo "== zlib =="
    [ -f zlib.tar.gz ] || curl -sL -o zlib.tar.gz "$ZLIBURL"
    rm -rf zlib-src && mkdir zlib-src
    tar xf zlib.tar.gz -C zlib-src --strip-components=1
    (cd zlib-src && CC="$CC" CFLAGS="-O2 -fPIE" ./configure --prefix="$SYSROOT" --static >/dev/null \
        && make -j"$(nproc)" >/dev/null && make install >/dev/null)
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
_ssl
_hashlib
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

if [ ! -f Makefile ]; then
    CC="$CC" \
    CFLAGS="-O2 -fPIE -fno-stack-protector -I$SYSROOT/include" \
    CPPFLAGS="-I$SYSROOT/include" \
    LDFLAGS="-static-pie -L$SYSROOT/lib" \
    ./configure \
        --prefix=/usr \
        --disable-shared \
        --without-ensurepip \
        --with-ensurepip=no \
        --disable-test-modules \
        --without-static-libpython \
        --disable-ipv6 \
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
strip -s "$STAGED/bin/python$PYSHORT" -o "$DEST/usr/bin/python3"

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
ls -la "$DEST/usr/bin/python3" "$DEST/usr/lib/python$PYTAG.zip"
echo ""
echo "fabriquer le disque :   ./mkdisk.sh $OUT"
echo "puis, sous l'OS     :   exec /usr/bin/python3 /mon-script.py"
