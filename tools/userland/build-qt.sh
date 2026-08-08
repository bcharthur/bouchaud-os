#!/bin/bash
# Construit Qt 5.15 en statique pour Bouchaud OS, et l'application de
# demonstration qui va avec.
#
# Resultat : `out-qt/qt-demo`, un binaire statique-PIE d'une quinzaine de Mio
# qui embarque Qt Core, Gui, Widgets et le plugin de plateforme linuxfb. A
# deposer sur le disque de donnees avec mkdisk.sh — le noyau n'a pas besoin
# d'etre recompile.
#
#   ./build-qt.sh          telecharge, construit Qt, puis la demonstration
#   ./build-qt.sh demo     reconstruit seulement la demonstration
#
# Compter une vingtaine de minutes pour Qt sur quatre cœurs.
#
# ## Pourquoi la glibc et non musl
#
# Le reste de l'userland est en musl, mais Qt est du C++ : il faudrait une
# libstdc++ construite contre musl, que la chaine `musl-tools` ne fournit pas.
# Le g++ du systeme, lui, apporte une libstdc++ complete — exceptions comprises.
# L'OS s'en moque : il implemente l'ABI de Linux, pas celle d'une libc en
# particulier. Un binaire glibc statique y tourne aussi bien qu'un binaire musl,
# ce qui a d'ailleurs ete verifie avant de s'engager dans cette voie.
#
# ## Pourquoi statique
#
# Un executable statique ne peut pas charger de `.so`, donc pas de plugin a
# decouvrir a l'execution : le plugin linuxfb est lie en dur par la macro
# `Q_IMPORT_PLUGIN` de qt-demo.cpp. C'est le mode nominal de Qt sur un systeme
# embarque, et c'est ce qui evite d'avoir a installer une arborescence de
# plugins au bon endroit.

set -e
cd "$(dirname "$0")"
ROOT=$PWD

QTVER=${QTVER:-5.15.13}
OUT=${OUT:-out-qt}
WORK=${WORK:-build-qt}
# Un chemin relatif est compris depuis ce repertoire ; un chemin absolu est
# garde tel quel, ce qui permet de construire ailleurs que dans les sources.
case "$WORK" in /*) ;; *) WORK=$ROOT/$WORK ;; esac
case "$OUT" in /*) ;; *) OUT=$ROOT/$OUT ;; esac
PREFIX=$WORK/install
SRC=$WORK/qtbase-everywhere-src-$QTVER
QTURL=${QTURL:-http://archive.ubuntu.com/ubuntu/pool/universe/q/qtbase-opensource-src/qtbase-opensource-src_${QTVER}+dfsg.orig.tar.xz}

# QtSvg est un module a part, absent de qtbase. Sans lui, `QImage` ne lit pas le
# SVG — et un site recent y met son logo, ses icones et ses pictogrammes. Ils
# manquaient tous a l'affichage, sans le moindre message.
SRC_SVG=$WORK/qtsvg-everywhere-src-$QTVER
SVGURL=${SVGURL:-http://archive.ubuntu.com/ubuntu/pool/universe/q/qtsvg-opensource-src/qtsvg-opensource-src_${QTVER}.orig.tar.xz}

# La source Ubuntu est expurgee de ses bibliotheques tierces embarquees (c'est
# le sens du suffixe `+dfsg`) : freetype, libpng, pcre2 et zlib doivent venir du
# systeme, en version statique.
verifie_dependances() {
    local manquantes=""
    for lib in libfreetype.a libpng.a libjpeg.a libpcre2-16.a libz.a libbrotlidec.a libbz2.a; do
        [ -f "/usr/lib/x86_64-linux-gnu/$lib" ] || manquantes="$manquantes $lib"
    done
    if [ -n "$manquantes" ]; then
        echo "bibliotheques statiques manquantes :$manquantes" >&2
        echo "  apt install libfreetype-dev libpng-dev libjpeg-dev libpcre2-dev zlib1g-dev libbrotli-dev libbz2-dev" >&2
        exit 1
    fi
}

construit_qt() {
    verifie_dependances
    mkdir -p "$WORK"
    cd "$WORK"

    [ -f qtbase.tar.xz ] || curl -fL -o qtbase.tar.xz "$QTURL"
    [ -d "qtbase-everywhere-src-$QTVER" ] || tar xf qtbase.tar.xz

    # Mkspec derive de linux-g++ : tout doit etre compilable en PIE, le binaire
    # final etant lie en static-pie (Bouchaud OS refuse une adresse fixe hors de
    # son creneau utilisateur, a 0x400000000000).
    local spec=$SRC/mkspecs/linux-g++-pie
    if [ ! -d "$spec" ]; then
        cp -r "$SRC/mkspecs/linux-g++" "$spec"
        cat >> "$spec/qmake.conf" <<'EOF'

# --- Bouchaud OS ---------------------------------------------------------
# Le binaire final est lie en static-pie : chaque objet, y compris ceux des
# bibliotheques statiques de Qt, doit etre independant de sa position.
QMAKE_CFLAGS   += -fPIE
QMAKE_CXXFLAGS += -fPIE
EOF
    fi

    mkdir -p build && cd build
    if [ ! -f Makefile ]; then
        echo "== configuration de Qt $QTVER =="
        # Tout ce qui suppose un systeme complet est coupe : ni OpenGL (aucun
        # GPU), ni X11 ni Wayland (il n'y a que le framebuffer), ni D-Bus, ni
        # fontconfig, ni glib. Reste linuxfb pour l'affichage et evdev pour les
        # entrees — exactement ce que l'OS sait fournir.
        "$SRC/configure" \
            -prefix "$PREFIX" \
            -platform linux-g++-pie \
            -static -release \
            -opensource -confirm-license \
            -qpa linuxfb -linuxfb -evdev \
            -no-opengl -no-egl -no-eglfs -no-gbm -no-kms -no-vulkan \
            -no-xcb -no-feature-xlib \
            -no-glib -no-dbus -no-icu -no-fontconfig -no-openssl -no-cups \
            -no-harfbuzz -no-libudev -no-libinput -no-mtdev -no-tslib \
            -system-zlib -system-libpng -system-libjpeg -system-freetype -system-pcre \
            -gif \
            -no-sql-sqlite -no-sql-mysql -no-sql-psql -no-sql-odbc \
            -nomake examples -nomake tests \
            -silent > configure.log 2>&1 || { tail -25 configure.log; exit 1; }
    fi

    echo "== compilation (une vingtaine de minutes) =="
    make -j"$(nproc)" > make.log 2>&1 || { grep -m5 -i "error" make.log; exit 1; }
    make install > install.log 2>&1
    cd "$ROOT"

    construit_svg
}

# QtSvg se construit apres qtbase, avec le qmake que celui-ci vient d'installer.
# Il produit `plugins/imageformats/libqsvg.a`, que le navigateur lie en dur
# comme les autres greffons d'image.
construit_svg() {
    if [ -f "$PREFIX/plugins/imageformats/libqsvg.a" ]; then
        return
    fi
    cd "$WORK"
    [ -f qtsvg.tar.xz ] || curl -fL -o qtsvg.tar.xz "$SVGURL" || {
        echo "qtsvg introuvable : le navigateur se passera du SVG" >&2
        cd "$ROOT"; return
    }
    [ -d "qtsvg-everywhere-src-$QTVER" ] || tar xf qtsvg.tar.xz

    echo "== module SVG =="
    mkdir -p build-svg && cd build-svg
    "$PREFIX/bin/qmake" "$SRC_SVG" > qmake.log 2>&1 || {
        echo "qmake a echoue sur qtsvg — voir $PWD/qmake.log" >&2
        cd "$ROOT"; return
    }
    make -j"$(nproc)" > make.log 2>&1 || { grep -m5 -i "error" make.log; cd "$ROOT"; return; }
    make install > install.log 2>&1
    cd "$ROOT"
}

construit_demo() {
    [ -x "$PREFIX/bin/qmake" ] || { echo "Qt n'est pas construit : lancer ./build-qt.sh" >&2; exit 1; }
    echo "== application de demonstration =="
    local chantier=$WORK/demo
    rm -rf "$chantier" && mkdir -p "$chantier"
    cp qt-demo.cpp "$chantier/"

    cat > "$chantier/qt-demo.pro" <<'EOF'
TEMPLATE = app
TARGET   = qt-demo
QT      += core gui widgets
CONFIG  += console static
CONFIG  -= app_bundle
SOURCES += qt-demo.cpp

# static-pie : l'OS charge le binaire ou il veut dans son creneau utilisateur.
QMAKE_LFLAGS   += -static-pie
QMAKE_CXXFLAGS += -fPIE

# libfreetype.a du systeme tire ses propres dependances statiques (brotli et
# bzip2, pour les polices compressees). `QMAKE_LIBS` et non `LIBS` : qmake place
# le premier en fin de ligne d'edition de liens, la ou l'editeur de liens
# statique a besoin de trouver les symboles encore manquants.
QMAKE_LIBS += -lbrotlidec -lbrotlicommon -lbz2
EOF

    (cd "$chantier" && "$PREFIX/bin/qmake" qt-demo.pro >/dev/null && make -j"$(nproc)" >/dev/null)

    rm -rf "$OUT" && mkdir -p "$OUT"
    strip -s "$chantier/qt-demo" -o "$OUT/qt-demo"
    echo ""
    ls -la "$OUT/qt-demo"
    echo ""
    echo "fabriquer le disque :   ./mkdisk.sh $OUT"
    echo "puis, sous l'OS     :   exec /qt-demo"
    echo ""
    echo "Note : Qt affiche au demarrage deux avertissements « iconv_open failed »."
    echo "Ils sont sans consequence — la glibc statique n'embarque pas ses modules"
    echo "de conversion gconv, et Qt retombe sur son codec interne."
}

case "${1:-tout}" in
    tout) construit_qt; construit_demo ;;
    demo) construit_demo ;;
    *) echo "usage: $0 [tout|demo]" >&2; exit 2 ;;
esac
