#!/usr/bin/env bash
# Repli de miroir APT apres un echec (BOUCHAUD_APT_REPLI_MIROIR_V1).
#
#   tools/ci/apt-repare-miroirs.sh
#
# Appele par apt-installe.sh APRES un essai rate, jamais avant : le chemin
# nominal ne change pas.
#
# Runs 43037630866 / 43037630856 / 43037630997 (dabbb7d, 11/11) : sur les
# runners `ubuntu24/20261101.360`, `apt-get update` restait sur
#   Ign: http://azure.archive.ubuntu.com/ubuntu noble InRelease   (toutes les 20 s)
#   Err: ... Temporary failure resolving 'azure.archive.ubuntu.com'
# La liste de miroirs de l'image (/etc/apt/apt-mirrors.txt, methode
# `mirror+file`) met le miroir Azure en tete, et le DNS du runner ne le
# resolvait plus ; trois essais rejouaient le meme miroir mort.
#
# Sur d'autres runners du meme run, le nom se resolvait mais le miroir ne
# repondait pas (21 a 42 `Ign:` en delai d'attente, aucune erreur DNS).
#
# Ce script DIT ce qu'il voit (liste des miroirs, resolution DNS et reponse
# de chacun), puis retire de la liste les seuls miroirs INJOIGNABLES -- nom
# non resolu, ou `dists/<version>/InRelease` sans reponse en 15 s --, en
# gardant les miroirs officiels Ubuntu que l'image y place deja. Il ne
# touche a aucun paquet, n'installe rien, et n'invente aucun succes : si plus
# aucun miroir ne se resout, il le dit et apt-installe.sh echouera.
set -uo pipefail
LISTE=${APT_LISTE_MIROIRS:-/etc/apt/apt-mirrors.txt}
VERSION=${APT_VERSION_UBUNTU:-$(sed -n 's/^VERSION_CODENAME=//p' /etc/os-release 2>/dev/null)}
VERSION=${VERSION:-noble}
echo "APT_DIAGNOSTIC liste=$LISTE" >&2
if [ ! -f "$LISTE" ]; then
  echo "APT_DIAGNOSTIC liste_absente (image sans methode mirror+file) : aucun repli" >&2
  exit 0
fi
sed 's/^/APT_DIAGNOSTIC miroir /' "$LISTE" >&2
garde=$(mktemp)
retires=0
gardes=0
while IFS= read -r ligne || [ -n "$ligne" ]; do
  url=${ligne%%[[:space:]]*}
  hote=$(printf '%s\n' "$url" | sed -E 's#^[a-z+]+://([^/:]+).*#\1#')
  if [ -z "$url" ] || [ "$hote" = "$url" ]; then
    printf '%s\n' "$ligne" >> "$garde"
    continue
  fi
  if ! getent hosts "$hote" >/dev/null 2>&1; then
    echo "APT_DIAGNOSTIC dns hote=$hote resolu=0" >&2
    retires=$((retires + 1))
    continue
  fi
  sonde="${url%/}/dists/$VERSION/InRelease"
  if curl -fsS -o /dev/null --max-time 15 "$sonde" 2>/dev/null; then
    echo "APT_DIAGNOSTIC dns hote=$hote resolu=1 repond=1" >&2
    printf '%s\n' "$ligne" >> "$garde"
    gardes=$((gardes + 1))
  else
    echo "APT_DIAGNOSTIC dns hote=$hote resolu=1 repond=0 sonde=$sonde" >&2
    retires=$((retires + 1))
  fi
done < "$LISTE"
if (( retires == 0 )); then
  echo "APT_REPLI_MIROIR aucun (tous les miroirs repondent : la panne est ailleurs)" >&2
elif (( gardes == 0 )); then
  echo "APT_REPLI_MIROIR impossible (aucun miroir ne repond)" >&2
else
  if [ -w "$LISTE" ]; then cp "$garde" "$LISTE"; else sudo cp "$garde" "$LISTE"; fi
  echo "APT_REPLI_MIROIR retires=$retires gardes=$gardes" >&2
  sed 's/^/APT_DIAGNOSTIC miroir_retenu /' "$LISTE" >&2
fi
rm -f "$garde"
exit 0
