#!/usr/bin/env bash
# P13_REVEIL_COEUR_MUET_V1 -- banc deterministe du reveil « perdu » entre la
# demande et la reprise.
#
#   tools/ci/run_reveil_coeur_muet.sh BOOTIMAGE [MASQUE_MS]
#
# Un fil noyau migrable dort un tick a la fois ; un fil epingle sur SON coeur
# masque les interruptions MASQUE_MS ms (3000 par defaut). Exige que le
# dormeur continue pendant le masquage :
#   * au moins un tour toutes les 10 ms en moyenne (sans le correctif : 0) ;
#   * aucun ecart entre deux tours superieur a 100 ms (sans : ~MASQUE_MS) ;
#   * ni double election ni incoherence `on_cpu` du dormeur ;
#   * un fil EPINGLE sur le meme coeur n'y est jamais vole, ne tourne pas
#     pendant le masquage, et reprend apres.
# Conditions de reproduction (sinon NON_QUALIFIANT, code 2) : SMP >= 2, le
# masquage a frappe le coeur du dormeur, un autre coeur etait au repos.
# Code 0 : OK ; 1 : ECHEC. Le banc n'a aucun seuil P13 : il oppose 0 tour a
# des centaines.
# BO_SMP (2 par defaut), BO_QEMU_KVM=1 : sous KVM.
#
# BANC_TENTATIVES=N (1 par defaut, 3 au plus) : protocole de remesure de
# run_ordonnanceur_attribution.sh. Un essai NON_QUALIFIANT (code 2 : la
# reproduction n'a pas ete atteinte -- en SMP2, masqueur sur le coeur un, le
# seul autre coeur est le BSP hors ordonnanceur) est REMESURE, jamais compte ;
# un ECHEC (code 1) ou un OK (code 0) arrete la serie et en est le verdict.
# N essais non qualifiants de suite : NON_QUALIFIANT (code 2). Chaque essai
# est garde (reveil-coeur-muet-tN.log).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_reveil_coeur_muet.sh BOOTIMAGE [MASQUE_MS]}
MASQUE=${2:-3000}
TENTATIVES=${BANC_TENTATIVES:-1}
case "$TENTATIVES" in 1|2|3) ;; *) echo "BANC_TENTATIVES=$TENTATIVES hors de 1..3" >&2; exit 1;; esac
if [ "$TENTATIVES" -gt 1 ]; then
  for t in $(seq 1 "$TENTATIVES"); do
    set +e
    BANC_TENTATIVES=1 "$PWD/tools/ci/run_reveil_coeur_muet.sh" "$BOOT" "$MASQUE"
    code=$?
    set -e
    cp reveil-coeur-muet.log "reveil-coeur-muet-t$t.log" 2>/dev/null || true
    echo "REVEIL_COEUR_MUET_TENTATIVE t=$t/$TENTATIVES code=$code"
    if [ "$code" != 2 ]; then exit "$code"; fi
  done
  echo "REVEIL_COEUR_MUET_NON_QUALIFIANT tentatives=$TENTATIVES (reproduction jamais atteinte)"
  exit 2
fi
SMP=${BO_SMP:-2}
ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
SCENARIO=scenario-reveil-coeur-muet
IMAGE=reveil-coeur-muet.img
LOG=reveil-coeur-muet.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG"
mkdir -p "$SCENARIO"
{
  echo "serie-dormeur-masque $MASQUE"
  echo 'serie-banc-attend dormeur'
  echo 'echo REVEIL_COEUR_MUET_AUTORUN_FIN'
} > "$SCENARIO/autorun"
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)
echo "REVEIL_COEUR_MUET smp=$SMP accel=$ACCEL masque_ms=$MASQUE"
: > "$LOG"
# shellcheck disable=SC2086
qemu-system-x86_64 $ACCEL -drive format=raw,file="$BOOT" -drive format=raw,file="$IMAGE" \
  -m 1024 -smp "$SMP" -display none -no-reboot ${BO_QEMU_EXTRA:-} -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 180))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE atteinte" >&2; break; fi
  grep -aqE 'KERNEL PANIC|panicked at' "$LOG" && { sleep 2; break; }
  grep -aqE 'SERIE_BANC_ATTEND_(OK|ECHEC) type=dormeur|REVEIL_COEUR_MUET_AUTORUN_FIN' "$LOG" && { sleep 1; break; }
  sleep 1
done
kill -TERM "$PID" 2>/dev/null || true; sleep 1; kill -KILL "$PID" 2>/dev/null || true; wait "$PID" 2>/dev/null || true

ligne=$(sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -ao 'SERIE_DORMEUR_MASQUE coeur=.*' | sed -n 1p || true)
champ() { echo "$ligne" | grep -oE "(^| )$1=[0-9]+" | sed -n '1s/.*=//p' || true; }
temoin=$(sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -ao 'BOUCHAUD_IMAGE_TEMOIN feature=[a-z0-9-]*' | sed 's/.*feature=//' | sort -u | tr '\n' ',' || true)
echo "IMAGE sha256=$(sha256sum "$BOOT" | cut -c1-64) build=$(sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -ao 'BOUCHAUD_BUILD commit=[^ ]*' | sed -n 1p | sed 's/.*commit=//') temoin=${temoin:-aucun}"
echo "${ligne:-SERIE_DORMEUR_MASQUE absent}"

# 1. Conditions de REPRODUCTION : sans elles le banc ne demontre rien.
#    NON_QUALIFIANT (code 2), jamais vert.
nq=""
[ "$SMP" -ge 2 ] || nq="$nq smp<2"
grep -aq 'SERIE_BANC_ATTEND_OK type=dormeur' "$LOG" || nq="$nq barriere-dormeur"
[ -n "$ligne" ] || nq="$nq ligne-absente"
[ "$(champ cible_atteinte)" = 1 ] || nq="$nq cible-non-atteinte"
[ "${ligne:+x}" = x ] && [ "$(champ autres_coeurs_au_repos)" -ge 1 ] 2>/dev/null || nq="$nq aucun-autre-coeur-au-repos"
grep -aqE 'KERNEL PANIC|panicked at' "$LOG" && nq=""  # une panique est un ECHEC, jamais un non-qualifiant
if [ -n "$nq" ] && ! grep -aqE 'KERNEL PANIC|panicked at' "$LOG"; then
  echo "REVEIL_COEUR_MUET_NON_QUALIFIANT raisons=${nq# }"
  exit 2
fi

# 2. Exigences du correctif.
tours=$(champ tours_pendant); ecart=$(champ ecart_max_ms)
echecs=0
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs=$((echecs + 1)); fi; }
exige "aucune panique noyau" bash -c "! grep -aqE 'KERNEL PANIC|panicked at' '$LOG'"
exige "le dormeur migrable a progresse pendant le masquage (>= 1 tour / 10 ms)" test "${tours:-0}" -ge $((MASQUE / 10))
exige "aucun ecart > 100 ms entre deux tours (pas de famine)" test "${ecart:-999999}" -le 100
exige "aucune double election du dormeur" test "$(champ doubles)" = 0
exige "on_cpu designe toujours le coeur qui execute le dormeur" test "$(champ incoherences)" = 0
exige "le fil epingle n'a jamais tourne hors de son coeur" test "$(champ epingle_hors_coeur)" = 0
exige "le fil epingle n'a pas tourne pendant le masquage de son coeur" test "$(champ epingle_tours_masque)" = 0
exige "le fil epingle a repris apres le masquage" test "$(champ epingle_tours_apres)" -ge 1
if [ "$echecs" -eq 0 ]; then echo "REVEIL_COEUR_MUET_OK tours=$tours ecart_max_ms=$ecart"; exit 0; fi
echo "REVEIL_COEUR_MUET_ECHEC n=$echecs tours=${tours:-?} ecart_max_ms=${ecart:-?}"
exit 1
