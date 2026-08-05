#!/bin/bash
# Construit les bibliotheques FFmpeg de decodage, en statique, pour Bouchaud OS.
#
#   ./build-ffmpeg.sh
#
# Produit `out-ffmpeg/lib/libav*.a` et les en-tetes, que `build-navigateur.sh`
# lie dans `bo-navigateur`.
#
# ## Pourquoi porter et non ecrire
#
# H.264 seul, c'est le codage arithmetique CABAC, le filtre de deblocage, la
# compensation de mouvement au quart de pixel, la prediction intra, les trames
# B hierarchiques et une demi-douzaine de profils. VP9 y ajoute son propre
# modele de probabilites et ses super-blocs. Ecrire cela a la main represente
# plusieurs annees-homme, et le resultat serait plus lent et moins juste que ce
# qui existe. On porte donc le decodeur de reference — c'est la reponse
# d'ingenierie, pas un renoncement.
#
# ## Ce qu'on garde
#
# Uniquement le decodage, et seulement les formats qui circulent reellement sur
# le web : H.264 et VP8/VP9 pour l'image, AAC, Opus, Vorbis et MP3 pour le son,
# dans des conteneurs MP4, Matroska/WebM et OGG. Pas d'encodeurs, pas de
# protocoles reseau (le moteur a deja les siens), pas de peripheriques de
# capture, pas de programmes en ligne de commande. La bibliotheque tombe ainsi
# a une taille compatible avec un binaire unique.
#
# ## Contraintes de la cible
#
# `-fPIE` partout : le binaire final est un static-pie. Pas de bibliotheque
# partagee, pas de `dlopen`, pas de threads systeme pour le decodage — l'OS sait
# faire des fils, mais un decodeur mono-fil est plus simple a raisonner et
# suffit a la demonstration.

set -e
cd "$(dirname "$0")"
ROOT=$PWD

OUT=${OUT:-out-ffmpeg}
WORK=${WORK:-build-ffmpeg}
VERSION=${VERSION:-6.1.1}
case "$WORK" in /*) ;; *) WORK=$ROOT/$WORK ;; esac
case "$OUT" in /*) ;; *) OUT=$ROOT/$OUT ;; esac

MIROIR=${MIROIR:-http://archive.ubuntu.com/ubuntu/pool/universe/f/ffmpeg}
ARCHIVE=ffmpeg_$VERSION.orig.tar.xz

mkdir -p "$WORK"
cd "$WORK"

if [ ! -d "ffmpeg-$VERSION" ]; then
    echo "== recuperation de FFmpeg $VERSION =="
    [ -f "$ARCHIVE" ] || curl -sfL --max-time 900 -o "$ARCHIVE" "$MIROIR/$ARCHIVE"
    tar xf "$ARCHIVE"
fi

SRC=$WORK/ffmpeg-$VERSION

if [ ! -f "$WORK/config.fait" ]; then
    echo "== configuration =="
    cd "$SRC"
    # `--disable-everything` puis on rallume nommement : c'est la seule facon
    # d'obtenir une bibliotheque dont on connaisse le contenu exact.
    ./configure \
        --prefix="$OUT" \
        --disable-everything \
        --disable-programs --disable-doc --disable-avdevice --disable-avfilter \
        --disable-postproc --disable-network --disable-protocols \
        --disable-encoders --disable-muxers --disable-filters --disable-bsfs \
        --disable-devices --disable-iconv --disable-xlib --disable-zlib \
        --disable-sdl2 --disable-vaapi --disable-vdpau --disable-videotoolbox \
        --disable-audiotoolbox --disable-cuda-llvm --disable-autodetect \
        --disable-shared --enable-static --enable-pic \
        --enable-decoder=h264,vp8,vp9,mjpeg,mpeg4,theora \
        --enable-decoder=aac,aac_latm,mp3,opus,vorbis,flac,pcm_s16le,pcm_u8 \
        --enable-demuxer=mov,matroska,webm_dash_manifest,ogg,mp3,wav,avi,aac,flac \
        --enable-parser=h264,vp8,vp9,aac,opus,vorbis,mpegaudio \
        --enable-protocol=file \
        --enable-swscale --enable-swresample \
        --extra-cflags="-fPIE -O2" --extra-ldflags="-fPIE" \
        --pkg-config-flags=--static \
        > "$WORK/configure.log" 2>&1 || { tail -30 "$WORK/configure.log"; exit 1; }
    touch "$WORK/config.fait"
    cd "$WORK"
fi

echo "== compilation (une dizaine de minutes) =="
cd "$SRC"
make -j"$(nproc)" > "$WORK/make.log" 2>&1 || { grep -m8 -iE "error" "$WORK/make.log"; exit 1; }

rm -rf "$OUT"
make install > "$WORK/install.log" 2>&1

echo ""
echo "pret dans $OUT/ :"
ls -la "$OUT"/lib/*.a | sed 's/^/  /'
du -sh "$OUT" | sed 's/^/  /'
