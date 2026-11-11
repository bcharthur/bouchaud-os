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
# Run 43054921395 (#297, 2cdab0a) : sur d'autres runners, AUCUN nom
# `*.ubuntu.com` ne se resolvait (azure.archive, archive, security), alors que
# packages.microsoft.com repondait : retirer des miroirs Ubuntu ne suffit
# pas. Second niveau : si aucun miroir de la liste ne repond, des miroirs
# publics Ubuntu HORS de ce domaine (APT_MIROIRS_SECOURS) sont sondes de meme
# et remplacent la liste. L'integrite ne change pas : APT verifie la
# signature `InRelease` avec les cles de l'archive Ubuntu, quel que soit le
# miroir -- ce sont les memes paquets signes.
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
SECOURS=${APT_MIROIRS_SECOURS:-http://mirrors.edge.kernel.org/ubuntu/ http://mirror.us.leaseweb.net/ubuntu/ http://ftp.uni-stuttgart.de/ubuntu/}

# repond URL : le nom se resout et `dists/<version>/InRelease` repond en 15 s.
repond() {
  local url=$1 hote
  hote=$(printf '%s\n' "$url" | sed -E 's#^[a-z+]+://([^/:]+).*#\1#')
  if ! getent hosts "$hote" >/dev/null 2>&1; then
    echo "APT_DIAGNOSTIC dns hote=$hote resolu=0" >&2
    return 1
  fi
  if curl -fsS -o /dev/null --max-time 15 "${url%/}/dists/$VERSION/InRelease" 2>/dev/null; then
    echo "APT_DIAGNOSTIC dns hote=$hote resolu=1 repond=1" >&2
    return 0
  fi
  echo "APT_DIAGNOSTIC dns hote=$hote resolu=1 repond=0 sonde=${url%/}/dists/$VERSION/InRelease" >&2
  return 1
}
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
  if [ -z "$url" ] || [ "${url#*://}" = "$url" ]; then
    printf '%s\n' "$ligne" >> "$garde"
    continue
  fi
  if repond "$url"; then
    printf '%s\n' "$ligne" >> "$garde"
    gardes=$((gardes + 1))
  else
    retires=$((retires + 1))
  fi
done < "$LISTE"
if (( retires == 0 )); then
  echo "APT_REPLI_MIROIR aucun (tous les miroirs repondent : la panne est ailleurs)" >&2
elif (( gardes == 0 )); then
  secours=$(mktemp)
  n=0
  for url in $SECOURS; do
    if repond "$url"; then
      n=$((n + 1))
      printf '%s\tpriority:%d\n' "$url" "$n" >> "$secours"
    fi
  done
  if (( n == 0 )); then
    echo "APT_REPLI_MIROIR impossible (aucun miroir ne repond, ni de la liste ni de secours)" >&2
  else
    if [ -w "$LISTE" ]; then cp "$secours" "$LISTE"; else sudo cp "$secours" "$LISTE"; fi
    echo "APT_REPLI_MIROIR secours=$n retires=$retires (paquets verifies par la signature de l'archive Ubuntu)" >&2
    sed 's/^/APT_DIAGNOSTIC miroir_retenu /' "$LISTE" >&2
  fi
  rm -f "$secours"
else
  if [ -w "$LISTE" ]; then cp "$garde" "$LISTE"; else sudo cp "$garde" "$LISTE"; fi
  echo "APT_REPLI_MIROIR retires=$retires gardes=$gardes" >&2
  sed 's/^/APT_DIAGNOSTIC miroir_retenu /' "$LISTE" >&2
fi
rm -f "$garde"
exit 0
