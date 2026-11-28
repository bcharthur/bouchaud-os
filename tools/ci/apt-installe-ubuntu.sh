#!/usr/bin/env bash
# Installe uniquement les paquets des sources Ubuntu signées du runner.
set -uo pipefail
[ "$#" -ge 1 ] || { echo "usage: apt-installe-ubuntu.sh PAQUET..." >&2; exit 2; }
ubuntu_sources=/etc/apt/sources.list.d/ubuntu.sources
[ -s "$ubuntu_sources" ] || { echo "APT_SOURCE_UBUNTU_ABSENTE $ubuntu_sources" >&2; exit 1; }
opts=(-o "Dir::Etc::sourcelist=$ubuntu_sources" -o Dir::Etc::sourceparts=-
      -o APT::Update::Error-Mode=any -o Acquire::Retries=3
      -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 -o DPkg::Lock::Timeout=120)
for essai in 1 2 3; do
  if sudo timeout 300 apt-get "${opts[@]}" update \
    && sudo timeout 900 apt-get "${opts[@]}" install -y --no-install-recommends "$@"; then
    exit 0
  fi
  echo "APT_UBUNTU_ESSAI_ECHEC essai=$essai paquets=$*" >&2
  if (( essai < 3 )); then sleep "$((essai * 15))"; fi
done
exit 1
