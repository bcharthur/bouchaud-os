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
case "$WORK" in /*) ;; *) WORK=$ROOT/$WORK ;; esac
case "$OUT" in /*) ;; *) OUT=$ROOT/$OUT ;; esac

[ -x "$QT/bin/qmake" ] || {
    echo "Qt statique introuvable dans $QT — lancer ./build-qt.sh" >&2
    exit 1
}
[ -f "$PY/usr/lib/libpython3.12.a" ] || {
    echo "libpython3.12.a introuvable dans $PY" >&2
    echo "  LIBC=glibc OUT=out-python-embed ./build-python.sh" >&2
    exit 1
}

rm -rf "$WORK" && mkdir -p "$WORK"
cp navigateur/hote.cpp "$WORK/"

cat > "$WORK/bo-navigateur.pro" <<EOF
TEMPLATE = app
TARGET   = bo-navigateur
QT      += core gui widgets
CONFIG  += console static
CONFIG  -= app_bundle
SOURCES += hote.cpp

INCLUDEPATH += $PY/usr/include

# static-pie : l'OS charge le binaire ou il veut dans son creneau utilisateur.
QMAKE_LFLAGS   += -static-pie
QMAKE_CXXFLAGS += -fPIE

# L'ordre compte : libpython vient avant les bibliotheques qu'elle reclame.
# OpenSSL est la pour le module \`ssl\` — c'est lui qui donne HTTPS au
# navigateur. brotli et bz2 sont les dependances statiques de freetype.
QMAKE_LIBS += $PY/usr/lib/libpython3.12.a $PY/usr/lib/libpythonaux.a \\
              -lssl -lcrypto -lutil -lm -lbrotlidec -lbrotlicommon -lbz2
EOF

(cd "$WORK" && "$QT/bin/qmake" bo-navigateur.pro >/dev/null && make -j"$(nproc)" >/dev/null)

# --- Empaquetage ------------------------------------------------------------
rm -rf "$OUT"
mkdir -p "$OUT/usr/lib" "$OUT/usr/share/bo-navigateur"
strip -s "$WORK/bo-navigateur" -o "$OUT/bo-navigateur"
cp "$PY/usr/lib/python312.zip" "$OUT/usr/lib/"
cp navigateur/navigateur.py navigateur/exemple-webview.py "$OUT/usr/share/bo-navigateur/"
cp -r navigateur/moteur "$OUT/usr/share/bo-navigateur/"

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
