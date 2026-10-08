#!/usr/bin/env bash
# Installe des paquets apt avec des delais bornes (BOUCHAUD_APT_BORNE_V1).
#
#   tools/ci/apt-installe.sh PAQUET...
#
# Run 37667817559 (e6799c24) : trois bancs Ladybird (smoke, sites, robustesse)
# et trois jobs Integration/Reliability sont restes 18 a 75 minutes dans
# `apt-get update` -- le miroir Azure ne repondait plus, et `apt-get` n'avait
# aucune borne -- avant d'etre annules par leur `timeout-minutes`, sans que le
# banc n'ait tourne. Ici : delai reseau de 30 s par requete, chaque appel borne,
# trois essais. Un echec reste un echec (code de sortie non nul) : le banc ne
# demarre pas sans ses outils.
set -uo pipefail
[ "$#" -ge 1 ] || { echo "usage: apt-installe.sh PAQUET..." >&2; exit 2; }
opts=(-o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 -o DPkg::Lock::Timeout=120)
for essai in 1 2 3; do
  if sudo timeout 300 apt-get "${opts[@]}" update \
    && sudo timeout 900 apt-get "${opts[@]}" install -y --no-install-recommends "$@"; then
    exit 0
  fi
  echo "APT_ESSAI_ECHEC essai=$essai paquets=$*" >&2
  sleep $((essai * 15))
done
echo "APT_ECHEC paquets=$*" >&2
exit 1
