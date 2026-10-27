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
#   * aucun ecart entre deux tours superieur a 100 ms (sans : ~MASQUE_MS).
# Le banc n'a aucun seuil P13 : il oppose 0 tour a des centaines.
# BO_SMP (2 par defaut), BO_QEMU_KVM=1 : sous KVM.
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_reveil_coeur_muet.sh BOOTIMAGE [MASQUE_MS]}
MASQUE=${2:-3000}
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
echo "${ligne:-SERIE_DORMEUR_MASQUE absent}"
tours=$(echo "$ligne" | grep -oE 'tours_pendant=[0-9]+' | cut -d= -f2 || true)
ecart=$(echo "$ligne" | grep -oE 'ecart_max_ms=[0-9]+' | cut -d= -f2 || true)
echecs=0
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs=$((echecs + 1)); fi; }
exige "aucune panique noyau" bash -c "! grep -aqE 'KERNEL PANIC|panicked at' '$LOG'"
exige "le masquage a vise le coeur du dormeur" grep -aq 'SERIE_BANC_ATTEND_OK type=dormeur' "$LOG"
exige "le dormeur a progresse pendant le masquage (>= 1 tour / 10 ms)" test "${tours:-0}" -ge $((MASQUE / 10))
exige "aucun ecart > 100 ms entre deux tours" test "${ecart:-999999}" -le 100
if [ "$echecs" -eq 0 ]; then echo "REVEIL_COEUR_MUET_OK tours=$tours ecart_max_ms=$ecart"; exit 0; fi
echo "REVEIL_COEUR_MUET_ECHEC n=$echecs tours=${tours:-?} ecart_max_ms=${ecart:-?}"
exit 1
