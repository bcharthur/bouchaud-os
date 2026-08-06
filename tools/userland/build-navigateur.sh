#!/bin/bash
# Construit le navigateur de Bouchaud OS : Qt + CPython dans un seul binaire.
#
#   ./build-navigateur.sh
#   ./mkdisk.sh out-navigateur
#   # puis, sous l'OS :
#   exec /bo-navigateur
#
# Prerequis, dans cet ordre :
#   ./build-qt.sh                      Qt 5.15 statique + plugin linuxfb
#   LIBC=glibc OUT=out-python-embed ./build-python.sh
#                                      libpython3.12.a + bibliotheque standard
#
# ## Une seule libc
#
# Qt est du C++ et tire la libstdc++ du systeme, donc la glibc. Python doit donc
# etre construit en glibc lui aussi : deux libc ne cohabitent pas dans un meme
# binaire. C'est la raison d'etre de la variante `LIBC=glibc` de build-python.sh.
#
# ## Un seul binaire
#
# Un executable statique ne peut charger ni plugin Qt ni extension Python. Les
# deux sont donc lies en dur : le plugin linuxfb par `Q_IMPORT_PLUGIN`, le module
# `bo` par `PyImport_AppendInittab`. En echange, il n'y a rien a installer : un
# fichier, et il tourne.

set -e
cd "$(dirname "$0")"
ROOT=$PWD

OUT=${OUT:-out-navigateur}
WORK=${WORK:-build-navigateur}
QT=${QT:-$ROOT/build-qt/install}
PY=${PY:-$ROOT/out-python-embed}
JS=${JS:-$ROOT/out-quickjs}
AV=${AV:-$ROOT/out-ffmpeg}
case "$WORK" in /*) ;; *) WORK=$ROOT/$WORK ;; esac
case "$OUT" in /*) ;; *) OUT=$ROOT/$OUT ;; esac
case "$JS" in /*) ;; *) JS=$ROOT/$JS ;; esac
case "$AV" in /*) ;; *) AV=$ROOT/$AV ;; esac

[ -x "$QT/bin/qmake" ] || {
    echo "Qt statique introuvable dans $QT — lancer ./build-qt.sh" >&2
    exit 1
}
[ -f "$PY/usr/lib/libpython3.12.a" ] || {
    echo "libpython3.12.a introuvable dans $PY" >&2
    echo "  LIBC=glibc OUT=out-python-embed ./build-python.sh" >&2
    exit 1
}
[ -f "$JS/lib/libquickjs.a" ] || {
    echo "libquickjs.a introuvable dans $JS — lancer ./build-quickjs.sh" >&2
    exit 1
}
[ -f "$AV/lib/libavcodec.a" ] || {
    echo "libavcodec.a introuvable dans $AV — lancer ./build-ffmpeg.sh" >&2
    exit 1
}

rm -rf "$WORK" && mkdir -p "$WORK"
cp navigateur/hote.cpp navigateur/bojs.cpp navigateur/bojs.h \
   navigateur/bomedia.cpp navigateur/bomedia.h "$WORK/"

cat > "$WORK/bo-navigateur.pro" <<EOF
TEMPLATE = app
TARGET   = bo-navigateur
QT      += core gui widgets
CONFIG  += console static
CONFIG  -= app_bundle
SOURCES += hote.cpp bojs.cpp bomedia.cpp

INCLUDEPATH += $PY/usr/include $JS/include $AV/include

# static-pie : l'OS charge le binaire ou il veut dans son creneau utilisateur.
QMAKE_LFLAGS   += -static-pie
QMAKE_CXXFLAGS += -fPIE

# L'ordre compte : libpython vient avant les bibliotheques qu'elle reclame.
# OpenSSL est la pour le module \`ssl\` — c'est lui qui donne HTTPS au
# navigateur. brotli et bz2 sont les dependances statiques de freetype.
# Les greffons d'image sont des archives statiques : `Q_IMPORT_PLUGIN` les
# declare, encore faut-il les fournir a l'editeur de liens.
GREFFONS = $QT/plugins/imageformats
# L'ordre des bibliotheques FFmpeg compte : avformat appelle avcodec, qui
# appelle avutil.
QMAKE_LIBS += $PY/usr/lib/libpython3.12.a $PY/usr/lib/libpythonaux.a \\
              $JS/lib/libquickjs.a \\
              $AV/lib/libavformat.a $AV/lib/libavcodec.a \\
              $AV/lib/libswscale.a $AV/lib/libswresample.a $AV/lib/libavutil.a \\
              \$\$GREFFONS/libqjpeg.a \$\$GREFFONS/libqgif.a \$\$GREFFONS/libqico.a \\
              -ljpeg \\
              -lssl -lcrypto -lutil -lm -lbrotlidec -lbrotlicommon -lbz2
EOF

(cd "$WORK" && "$QT/bin/qmake" bo-navigateur.pro >/dev/null && make -j"$(nproc)" >/dev/null)

# --- Empaquetage ------------------------------------------------------------
rm -rf "$OUT"
mkdir -p "$OUT/usr/lib" "$OUT/usr/share/bo-navigateur"
strip -s "$WORK/bo-navigateur" -o "$OUT/bo-navigateur"
cp "$PY/usr/lib/python312.zip" "$OUT/usr/lib/"
cp navigateur/navigateur.py navigateur/exemple-webview.py \
   navigateur/test_moteur.py navigateur/media-probe.py navigateur/persist-probe.py navigateur/youtube-probe.py "$OUT/usr/share/bo-navigateur/"
cp navigateur/demo-js.html navigateur/demo-media.html navigateur/demo-h264.html \
   "$OUT/usr/share/bo-navigateur/"
# La video et le son de la demonstration sont fabriques ici : les
# telecharger ferait dependre la demo du reseau et de droits d'usage.
if command -v cjpeg >/dev/null 2>&1; then
    python3 fabrique-media-demo.py "$OUT/usr/share/bo-navigateur" || true
    # Un vrai flux H.264 + AAC, de la forme d'un flux progressif YouTube
    # (itag 18). Il faut un encodeur, que seule la machine de construction a.
    if command -v ffmpeg >/dev/null 2>&1; then
        ffmpeg -y -f lavfi -i "testsrc2=size=480x270:rate=15:duration=8" \
               -f lavfi -i "sine=frequency=440:duration=8" \
               -c:v libx264 -profile:v baseline -level 3.0 -pix_fmt yuv420p \
               -b:v 250k -c:a aac -b:a 64k -ar 44100 -ac 2 -movflags +faststart \
               "$OUT/usr/share/bo-navigateur/demo-h264.mp4" >/dev/null 2>&1 \
            && echo "  demo-h264.mp4 : H.264 baseline + AAC"
    fi
else
    echo "  cjpeg absent : la video de demonstration n'est pas fabriquee"
fi
cp -r navigateur/moteur "$OUT/usr/share/bo-navigateur/"
# `prelude.js` est du code, pas une ressource optionnelle : sans lui le
# JavaScript d'une page ne trouve ni `document` ni `setTimeout`.
[ -f "$OUT/usr/share/bo-navigateur/moteur/prelude.js" ] || {
    echo "prelude.js manquant dans le paquet" >&2
    exit 1
}

# pywebview et son moteur Bouchaud OS : c'est ce qui permet de faire tourner du
# code pywebview sans le modifier.
CHANTIER="$WORK/pywebview" navigateur/greffe-pywebview.sh "$OUT/usr/lib/python3/site-packages"

find "$OUT/usr" -name '__pycache__' -type d -exec rm -rf {} + 2>/dev/null || true

# Racines de certification, si la machine de construction en a : sans elles, le
# navigateur accepte les certificats HTTPS sans les verifier et le dit dans sa
# barre d'etat.
for magasin in /etc/ssl/certs/ca-certificates.crt /etc/pki/tls/certs/ca-bundle.crt; do
    if [ -f "$magasin" ]; then
        mkdir -p "$OUT/etc/ssl/certs"
        cp "$magasin" "$OUT/etc/ssl/certs/ca-certificates.crt"
        break
    fi
done

echo ""
echo "pret dans $OUT/ :"
du -sh "$OUT" | sed 's/^/  /'
ls -la "$OUT/bo-navigateur" | sed 's/^/  /'
echo ""
echo "fabriquer le disque :   ./mkdisk.sh $OUT"
echo "puis, sous l'OS     :   exec /bo-navigateur [url]"
