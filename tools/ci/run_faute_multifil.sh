#!/usr/bin/env bash
# EXCEPTION RING 3 DANS UN PROCESSUS MULTI-FILS (BOUCHAUD_FAUTE_MULTIFIL_V1).
#
#   tools/ci/run_faute_multifil.sh BOOTIMAGE [TOURS]
#
# Un demarrage, une sonde : faute-multifil-probe (tools/userland), qui fait
# fauter (#UD, #DE, #GP, #PF) un processus a quatre fils -- depuis le fil
# principal puis depuis un frere -- pendant que les autres dorment, calculent
# ou attendent dans read(). Le noyau doit tuer le processus et SURVIVRE.
#
# C'est la preuve QEMU du Crash Guard P19 (BOUCHAUD_P19_FATAL_USER_IRQ_EXIT_V1) :
# un noyau qui appelle `exit_group` avec IF=0 depuis le gestionnaire panique
# ici des la premiere faute #UD (l'attente des freres passe par schedule()).
#
# BO_QEMU_KVM=1 : meme demarrage sous KVM. BO_SMP : nombre de coeurs (4).
set -euo pipefail
cd "$(dirname "$0")/../.."
BOOT=${1:?usage: run_faute_multifil.sh BOOTIMAGE [TOURS]}
TOURS=${2:-3}
SMP=${BO_SMP:-4}

SCENARIO=scenario-faute-multifil
IMAGE=faute-multifil.img
LOG=serie-faute-multifil.log
rm -rf "$SCENARIO" "$IMAGE" "$LOG" tools/userland/out-faute-multifil
(cd tools/userland && OUT=out-faute-multifil ./build.sh musl >/dev/null)
mkdir -p "$SCENARIO/bin"
cp tools/userland/out-faute-multifil/faute-multifil-probe "$SCENARIO/bin/"
printf '/bin/faute-multifil-probe %s\necho FAUTE_MULTIFIL_FIN\n' "$TOURS" > "$SCENARIO/autorun"
(cd tools/userland && IMAGE="$PWD/../../$IMAGE" ./mkdisk.sh "$PWD/../../$SCENARIO" >/dev/null)

ACCEL="-cpu max"
if [ "${BO_QEMU_KVM:-0}" = 1 ]; then
  [ -w /dev/kvm ] || { echo "BO_QEMU_KVM=1 mais /dev/kvm inaccessible" >&2; exit 1; }
  ACCEL="-enable-kvm -cpu host"
fi
echo "FAUTE_MULTIFIL_ACCEL ${ACCEL} smp=${SMP} tours=${TOURS}"
: > "$LOG"
# shellcheck disable=SC2086
qemu-system-x86_64 $ACCEL \
  -drive format=raw,file="$BOOT" \
  -drive format=raw,file="$IMAGE" \
  -m 2048 -smp "$SMP" -display none -no-reboot \
  -serial file:"$LOG" &
PID=$!
LIMITE=$((SECONDS + 600))
while kill -0 "$PID" 2>/dev/null; do
  if (( SECONDS >= LIMITE )); then echo "ECHEANCE atteinte" >&2; break; fi
  # La sortie de `echo`, pas sa trace (`+ echo FAUTE_MULTIFIL_FIN`).
  if grep -aqE 'KERNEL PANIC|panicked at' "$LOG" \
     || sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -a 'FAUTE_MULTIFIL_FIN' | grep -av '+ echo FAUTE_MULTIFIL_FIN' >/dev/null; then
    sleep 2; break
  fi
  sleep 1
done
kill -TERM "$PID" 2>/dev/null || true
sleep 1
kill -KILL "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true

propre() { sed -E 's/\x1b\[[0-9;]*m//g; s/^\[[^]]*\]\[[^]]*\]\[FPS:[^]]*\] //' "$LOG"; }
propre | grep -aE '^  tour=|FAUTE_MULTIFIL_|PROCESS_FAULT |KERNEL PANIC|panicked at|EXTINCTION_FRERES_LENTE' | head -80 || true
propre | grep -aA6 'panicked at' | head -12 || true

echecs=()
exige() { local quoi=$1; shift; if "$@"; then echo "  ok      $quoi"; else echo "  ECHEC   $quoi"; echecs+=("$quoi"); fi; }
exige "aucune panique noyau" bash -c "! grep -aqE 'KERNEL PANIC|panicked at' '$LOG'"
exige "chaque fils fautif tue par un signal (FAUTE_MULTIFIL_OK)" grep -aqE "FAUTE_MULTIFIL_OK essais=$((TOURS * 8))" "$LOG"
exige "le noyau a vu des fautes ring 3 (PROCESS_FAULT)" grep -aq 'PROCESS_FAULT pid=' "$LOG"
exige "l'autorun continue apres la sonde" grep -aq 'FAUTE_MULTIFIL_FIN' "$LOG"
if [ ${#echecs[@]} -eq 0 ]; then
  echo "FAUTE_MULTIFIL_BANC_OK"
else
  echo "FAUTE_MULTIFIL_BANC_ECHEC n=${#echecs[@]}"
  exit 1
fi
