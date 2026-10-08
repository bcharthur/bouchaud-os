#!/usr/bin/env bash
# A/B DU COUT DU PROFIL RIP (BOUCHAUD_PROFIL_RIP_V1).
#
#   tools/ci/run_ab_profil.sh BOOTIMAGE NATIVE_DIR [DUREE_S]
#
# Meme image, meme banc, meme fixture, meme executant, meme duree : deux
# endurances l'une apres l'autre, profil RIP actif (A) puis coupe (B). Un outil
# de diagnostic ne doit pas changer ce qu'il mesure ; on compare les cycles,
# les latences d'un cadre (p50/p95/p99), le retard de la boucle de la page, le
# rythme des appels systeme, le CPU et le RSS des services, et l'on publie
# `PROFIL_RIP_OVERHEAD <metrique>=<ecart %>`.
#
# Verdict : la STABILITE est fonctionnelle dans les deux bras (aucune panique,
# assertion, faute, cycle non conclu) -- un STABILITY_GATE en echec dans l'un
# ou l'autre rend 1. Le PERFORMANCE_GATE de chaque bras et l'ecart A/B sont
# imprimes, pas juges ici : c'est la mesure. Une ligne FIN absente rend 1.
set -uo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_ab_profil.sh BOOTIMAGE NATIVE_DIR [DUREE_S]}
OUT=${2:?usage: run_ab_profil.sh BOOTIMAGE NATIVE_DIR [DUREE_S]}
DUREE=${3:-300}
for bras in on off; do
  echo "=== bras profil-rip=$bras ($DUREE s) ==="
  BO_ENDURANCE_PROFIL_RIP=$bras tools/ci/run_ladybird_endurance.sh "$BOOT" "$OUT" "$DUREE" > "ab-profil-$bras.sortie" 2>&1
  echo "  rc=$? $(grep -aE '^(STABILITY_GATE|PERFORMANCE_GATE|LADYBIRD_ENDURANCE_(OK|ECHEC))' "ab-profil-$bras.sortie" | tr '\n' ' ')"
  cp serie-endurance.log "serie-endurance-ab-$bras.log" 2>/dev/null
done
rc=0
python3 tools/ci/compare_ab_profil.py ab-profil-on.sortie ab-profil-off.sortie || rc=1
for bras in on off; do
  if ! grep -aq '^STABILITY_GATE ok' "ab-profil-$bras.sortie"; then
    echo "AB_PROFIL_STABILITE_ECHEC bras=$bras"
    rc=1
  fi
done
[ "$rc" -eq 0 ] && echo "AB_PROFIL_STABILITE_OK"
exit "$rc"
