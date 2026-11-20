#!/usr/bin/env bash
# Reproduit l’archive vcpkg depuis les objets Git du zlib Chromium épinglé.
set -euo pipefail
cd "$(dirname "$0")/../.."
if [[ $# -gt 2 ]]; then
    echo 'usage: prepare_angle_zlib_cache.sh [DEPOT_GIT] [CACHE_VCPKG]' >&2
    exit 2
fi
source_git=${1:-https://chromium.googlesource.com/chromium/src/third_party/zlib}
cache=${2:-third_party/vcpkg-downloads}
revision=4028ebf8710ee39d2286cb0f847f9b95c59f84d8
port=third_party/vcpkg-gfx/ports/angle/portfile.cmake
if ! grep -Fxq "set(ANGLE_THIRDPARTY_ZLIB_COMMIT $revision)" "$port"; then
    echo 'Le pin ANGLE diffère de celui attendu dans le port vcpkg.' >&2
    exit 2
fi
mkdir -p -- "$cache"
travail=$(mktemp -d "$cache/.angle-zlib.XXXXXXXX")
trap 'rm -rf -- "$travail"' EXIT
git init -q --bare "$travail/git"
git --git-dir="$travail/git" -c fetch.fsckObjects=true fetch --no-tags --depth=1 -- "$source_git" "$revision"
verifie=$(git --git-dir="$travail/git" rev-parse --verify 'FETCH_HEAD^{commit}')
if [[ $verifie != "$revision" ]]; then
    echo "Révision zlib inattendue $verifie." >&2
    exit 1
fi
git --git-dir="$travail/git" fsck --strict --no-reflogs "$revision"
archive="$cache/angle-$revision.tar.gz"
git --git-dir="$travail/git" -c core.autocrlf=false archive "$revision" -o "$travail/zlib.tar.gz"
if ! ln -- "$travail/zlib.tar.gz" "$archive"; then
    if ! cmp -s -- "$travail/zlib.tar.gz" "$archive"; then
        echo "Archive différente ou inaccessible dans le cache $archive." >&2
        exit 1
    fi
fi
sha256sum -- "$archive"
