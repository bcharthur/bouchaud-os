#!/bin/bash
# Decrit une image userland : d'ou elle vient, et ce qu'elle contient.
#
#   ./manifeste.sh [image] > userland-manifest.json
#
# ## Pourquoi un manifeste
#
# `userland.img` est un fichier opaque de plusieurs dizaines de mebioctets. Rien
# en lui ne dit de quel commit il provient, ni quelle version de Qt il embarque.
# Tant qu'un seul developpeur le construisait sur sa machine, la question ne se
# posait pas : l'image etait forcement celle du travail en cours.
#
# Des l'instant ou une machine **telecharge** son userland au lieu de le
# construire, elle doit pouvoir refuser ce qui ne correspond pas. Un userland
# d'un autre commit ne se signale pas : il demarre, et il se comporte comme
# l'image d'un autre jour — un binaire qui ne connait pas le dernier appel
# systeme, un moteur qui ne connait pas le dernier protocole. La panne qui suit
# accuse le code source, qui n'y est pour rien.
#
# Le manifeste rend donc explicite ce que l'image ne peut pas dire d'elle-meme,
# et `run.ps1` en fait une condition de demarrage : bon commit, bonne empreinte,
# ou pas de userland du tout.

set -e
cd "$(dirname "$0")"
ROOT=$PWD

IMAGE=${1:-userland.img}
[ -f "$IMAGE" ] || { echo "manifeste: '$IMAGE' introuvable" >&2; exit 1; }

# Les versions viennent des **scripts de construction**, pas d'une liste tenue
# a cote. Une liste separee aurait derive au premier changement de version que
# personne n'aurait pense a y reporter, et le manifeste aurait alors menti avec
# aplomb.
versionne() {
    # `versionne <script> <variable> <defaut>` : lit `VAR=${VAR:-valeur}`.
    sed -n "s/^$2=\${$2:-\([^}]*\)}.*/\1/p" "$1" | head -1
}

QTVER=$(versionne build-qt.sh QTVER)
PYVER=$(versionne build-python.sh PYVER)
AVVER=$(versionne build-ffmpeg.sh VERSION)
QJSPYPI=$(versionne build-quickjs.sh PYPI_VER)

# QuickJS n'a pas de numero de version au sens habituel : il porte une date,
# ecrite par `build-quickjs.sh` dans le paquet qu'il produit. On la lit la
# plutot que de la deviner depuis la version du paquet PyPI qui l'embarque.
QJSVER=$(cat out-quickjs/VERSION 2>/dev/null || echo "inconnu")

COMMIT=$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo "inconnu")
BRANCHE=$(git -C "$ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || echo "inconnu")
# Un arbre modifie donne une image qui ne correspond a aucun commit publie.
# Le dire est le minimum : une image marquee `sale` ne doit jamais servir de
# reference a quelqu'un d'autre.
if [ -n "$(git -C "$ROOT" status --porcelain 2>/dev/null)" ]; then
    PROPRE=false
else
    PROPRE=true
fi

TAILLE=$(wc -c < "$IMAGE" | tr -d ' ')
SOMME=$(sha256sum "$IMAGE" | cut -d' ' -f1)
QUAND=$(date -u +%Y-%m-%dT%H:%M:%SZ)

LADYBIRD_M5=false
if tar -tf "$IMAGE" 2>/dev/null | grep -qE '(^|\./)usr/libexec/ladybird/libipc-codec-probe$'; then
    LADYBIRD_M5=true
fi

cat <<JSON
{
  "schema": 1,
  "git_commit": "$COMMIT",
  "git_branch": "$BRANCHE",
  "git_clean": $PROPRE,
  "built_at": "$QUAND",
  "image": "$(basename "$IMAGE")",
  "image_size": $TAILLE,
  "sha256": "$SOMME",
  "qt_version": "$QTVER",
  "python_version": "$PYVER",
  "quickjs_version": "$QJSVER",
  "quickjs_pypi_version": "$QJSPYPI",
  "ffmpeg_version": "$AVVER",
  "ladybird_m5_probe": $LADYBIRD_M5
}
JSON
