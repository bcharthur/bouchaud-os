#!/bin/bash
# Construit tout ce qu'il faut pour que l'OS ait un navigateur, dans l'ordre.
#
#   ./build-tout.sh              tout ce qui manque, puis le disque
#   ./build-tout.sh --force      tout reconstruire
#
# Sous WSL ou Linux. Rien de cette chaine ne tourne sous Windows : elle compile
# du C, du C++ et du Python pour une cible Linux-ABI.
#
# ## Pourquoi ce script existe
#
# `/bo-navigateur` est un ELF unique qui embarque Qt, CPython, QuickJS et
# FFmpeg. Aucun de ces quatre n'est versionne — ils se telechargent et se
# construisent —, et `build-navigateur.sh` refuse de demarrer si l'un manque.
# La chaine correcte tenait donc dans la tete de celui qui l'avait ecrite, avec
# ses variables d'environnement et son ordre : `LIBC=glibc` pour Python parce
# que Qt est du C++, FFmpeg avant le navigateur parce qu'il lie `libavcodec.a`.
# Une etape oubliee ne se voyait qu'a la fin, sous la forme d'un OS qui demarre
# et annonce `/bo-navigateur absent`.
#
# ## Ce que cela coute
#
# Une heure environ sur quatre cœurs la premiere fois, Qt en representant les
# deux tiers. Chaque etape est sautee si son resultat est deja la, donc une
# seconde execution ne recompile que ce qui manque.
#
#   quickjs     ~1 min      moteur JavaScript
#   ffmpeg      ~15 min     decodeurs audio/video
#   python      ~10 min     interprete embarque (glibc, pour Qt)
#   qt          ~20-40 min  Qt 5.15 statique + plugin linuxfb
#   navigateur  ~2 min      l'ELF unique
#   disque      instantane  userland.img
#
# ## Ce qu'il faut avoir avant
#
#   apt install build-essential python3-dev libbrotli-dev perl pkg-config \
#               libfontconfig1-dev libfreetype-dev nasm curl tar xz-utils

set -e
cd "$(dirname "$0")"
ROOT=$PWD

FORCE=0
[ "$1" = "--force" ] && FORCE=1

etape() {
    # `etape <nom> <temoin> <commande...>` : saute si le temoin existe deja.
    nom=$1; temoin=$2; shift 2
    if [ "$FORCE" -eq 0 ] && [ -e "$temoin" ]; then
        echo "== $nom : deja construit ($temoin)"
        return 0
    fi
    echo ""
    echo "== $nom =="
    "$@"
}

etape "QuickJS"  "$ROOT/out-quickjs/lib/libquickjs.a"        ./build-quickjs.sh
etape "FFmpeg"   "$ROOT/out-ffmpeg/lib/libavcodec.a"         ./build-ffmpeg.sh

# `LIBC=glibc` n'est pas un detail : Qt est du C++ et tire la libstdc++ du
# systeme, donc la glibc. Un Python musl et un Qt glibc ne se lient pas dans le
# meme binaire, et l'erreur n'apparait qu'a l'edition de liens du navigateur —
# tout au bout, apres quarante minutes de Qt.
etape "CPython"  "$ROOT/out-python-embed/usr/lib/libpython3.12.a" \
      env LIBC=glibc OUT=out-python-embed ./build-python.sh

etape "Qt"       "$ROOT/build-qt/install/bin/qmake"          ./build-qt.sh
etape "Navigateur" "$ROOT/out-navigateur/bo-navigateur"      ./build-navigateur.sh

# Artefact de transition M5 : le binaire est compile pour la cible par la chaine
# Ladybird, puis livre dans le vrai userland interactif. Il ne tourne jamais sur
# l'hote Linux : seule la compilation croisee y a lieu.
if [ -n "${BO_LADYBIRD_M5_PROBE:-}" ]; then
    [ -x "$BO_LADYBIRD_M5_PROBE" ] || {
        echo "Ladybird M5: probe cible introuvable ou non executable: $BO_LADYBIRD_M5_PROBE" >&2
        exit 1
    }
    DEST="$ROOT/out-navigateur/usr/libexec/ladybird"
    mkdir -p "$DEST"
    cp "$BO_LADYBIRD_M5_PROBE" "$DEST/libipc-codec-probe"
    chmod 755 "$DEST/libipc-codec-probe"
    strip --strip-unneeded "$DEST/libipc-codec-probe" 2>/dev/null || true
    echo "== Ladybird M5 embarque : /usr/libexec/ladybird/libipc-codec-probe"
    ls -lh "$DEST/libipc-codec-probe"
fi

echo ""
echo "== disque userland =="
./mkdisk.sh out-navigateur

# Le manifeste voyage avec l'image, meme construite ici. Sans lui, `run.ps1` ne
# peut pas distinguer un userland construit pour ce commit d'un vestige d'il y a
# trois semaines — et il retelechargerait a chaque demarrage, ou pire, garderait
# le vestige.
./manifeste.sh userland.img > userland-manifest.json

echo ""
echo "pret :"
ls -la "$ROOT/userland.img" | sed 's/^/  /'
sed -n 's/^  "\(git_commit\|qt_version\|python_version\|ffmpeg_version\|sha256\)": /  \1 /p' \
    userland-manifest.json
echo ""
echo "Relancer .\\run.ps1 (ou run.ps1) : le second disque sera attache"
echo "automatiquement, et le bureau trouvera /bo-navigateur."
