#!/usr/bin/env bash
#
# BOUCHAUD_SYSRQ_RELEVE_FILS_V1 (#283) : le releve des fils par Alt+SysRq,
# eprouve sur le VRAI noyau sous QEMU.
#
#   tools/ci/run_releve_fils_e2e.sh BOOTIMAGE [JOURNAL]
#
# Ce que le banc exige, et pourquoi :
#   1. rien n'est publie SPONTANEMENT : le releve coute des centaines de lignes
#      sur un navigateur ; il ne doit sortir que sur demande ;
#   2. Alt SEUL ne declenche rien (le detecteur est dans l'IRQ1, sur le chemin
#      de chaque touche tapee) ;
#   3. `sendkey alt-sysrq` sur le moniteur publie un releve COMPLET : debut,
#      une ligne `[FIL-FORENSIQUE]` par fil annonce, fin -- c'est exactement
#      ce que les bancs Ladybird demandent a une VM figee.
# Tout manquement est un ECHEC (code 1).
set -uo pipefail
cd "$(dirname "$0")/../.."

BOOT=${1:?usage: run_releve_fils_e2e.sh BOOTIMAGE [JOURNAL]}
LOG=${2:-releve-fils.log}
MONITEUR=$PWD/moniteur-releve-fils.sock
[ -s "$BOOT" ] || { echo "RELEVE_FILS_E2E_ECHEC raison=bootimage-absente"; exit 1; }

: > "$LOG"
rm -f "$MONITEUR"
qemu-system-x86_64 \
  -drive format=raw,file="$BOOT" \
  -m 2048 -smp 2 -display none -no-reboot \
  -monitor "unix:$MONITEUR,server,nowait" \
  -serial file:"$LOG" &
PID=$!
trap 'kill -TERM "$PID" 2>/dev/null; sleep 1; kill -KILL "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; rm -f "$MONITEUR"' EXIT

echecs=0
echec() { echo "  ECHEC   $1"; echecs=$((echecs + 1)); }
ok() { echo "  ok      $1"; }

# Le fil diagnostique est vivant des que le premier resume periodique sort.
pret=0
for _ in $(seq 1 120); do
  kill -0 "$PID" 2>/dev/null || break
  if grep -aq '\[SCHED-RESUME\]' "$LOG"; then pret=1; break; fi
  sleep 2
done
if (( pret == 0 )); then
  echo "RELEVE_FILS_E2E_ECHEC raison=noyau_pas_pret"
  tail -c 3000 "$LOG" | sed -E 's/\x1b\[[0-9;]*m//g' | tail -15
  exit 1
fi

compte() { grep -ac "$1" "$LOG" || true; }

[ "$(compte '\[FIL-RELEVE\] debut')" = 0 ] && ok "aucun releve spontane au demarrage" \
  || echec "un releve est sorti sans demande"

# Alt seul.
python3 - "$MONITEUR" <<'PY'
import socket, sys, time
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(5)
s.connect(sys.argv[1])
try:
    s.recv(4096)
except socket.timeout:
    pass
s.sendall(b"sendkey alt\n")
time.sleep(0.5)
s.close()
PY
sleep 5
[ "$(compte '\[FIL-RELEVE\] debut')" = 0 ] && ok "Alt seul ne declenche rien" \
  || echec "Alt seul a declenche un releve"

sortie=$(tools/ci/releve_fils_qemu.sh "$MONITEUR" "$LOG" 30)
echo "  $sortie"
case "$sortie" in
  FIL_RELEVE_OBTENU*) ok "Alt+SysRq publie un releve" ;;
  *) echec "Alt+SysRq n'a publie aucun releve" ;;
esac

propre=$(sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | tr -d '\r')
annonces=$(echo "$propre" | grep -aoE '\[FIL-RELEVE\] fin n=1 fils=[0-9]+' | grep -oE '[0-9]+$' | head -1 || true)
lignes=$(echo "$propre" | awk '/\[FIL-RELEVE\] debut n=1 /,/\[FIL-RELEVE\] fin n=1 / { if (/\[FIL-FORENSIQUE\] pid=[0-9]+ tid=[0-9]+ etat=/) n++ } END { print n + 0 }')
if [ -n "$annonces" ] && (( annonces >= 1 )) && [ "$annonces" = "$lignes" ]; then
  ok "releve complet : $lignes fil(s) decrit(s) sur $annonces annonce(s)"
else
  echec "releve incomplet : ${lignes} ligne(s) pour ${annonces:-?} fil(s) annonce(s)"
fi
[ "$(compte '\[FIL-RELEVE\] debut')" = 1 ] && ok "une demande, un releve" \
  || echec "$(compte '\[FIL-RELEVE\] debut') releve(s) pour une demande"

if (( echecs == 0 )); then
  echo "RELEVE_FILS_E2E_OK fils=$lignes"
  exit 0
fi
echo "RELEVE_FILS_E2E_ECHEC n=$echecs"
exit 1
